//! Background tasks and loops in a Claude Code transcript.
//!
//! A task starts when the result of a `Bash`, `Agent`, `Task` or `Monitor` call names a task id
//! (`backgroundTaskId`, or `agentId` of an async agent) and ends with a `<task-notification>` for
//! that id or a `TaskStop`/`KillShell` call. A loop is a `CronCreate` that lasts until `CronDelete`,
//! or a `ScheduleWakeup` that stands until the next one replaces it.

use super::super::{BgTask, LoopInfo, Signals};
use super::transcript::result_text;
use crate::activity::one_line;
use crate::agents::command_label;
use crate::util::iso_to_ms;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

/// Calls whose result has not been read, tasks and loops kept at most.
const CALLS_CAP: usize = 64;
const TASKS_CAP: usize = 32;
const LOOPS_CAP: usize = 16;
/// The key of the one self-paced loop a session has.
const WAKE_KEY: &str = "wakeup";

static RE_NOTICE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<task-id>\s*([^<\s]+)\s*</task-id>").expect("static regex"));
static RE_NOTICE_STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<status>\s*([^<\s]+)\s*</status>").expect("static regex"));
static RE_SHELL_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Command running in background with ID:\s*([A-Za-z0-9_-]+)").expect("static regex")
});
static RE_AGENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"agentId:\s*([A-Za-z0-9_-]+)").expect("static regex"));

/// A call of interest whose result is still to come.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Call {
    Task { kind: String, label: String },
    Cron { cron: String, prompt: String },
}

/// What a read hands to the next one.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    calls: Vec<(String, Call)>,
    #[serde(default)]
    tasks: Vec<BgTask>,
    #[serde(default)]
    loops: Vec<(String, LoopInfo)>,
}

/// Follows the lines of one transcript.
#[derive(Default)]
pub struct Tracker {
    state: State,
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// A cron expression in words where that is easy, else the expression itself.
fn describe_cron(cron: &str) -> String {
    let f: Vec<&str> = cron.split_whitespace().collect();
    let [min, hour, dom, month, dow] = f[..] else {
        return cron.trim().to_string();
    };
    let num = |x: &str| x.parse::<u32>().ok();
    let every = |x: &str| x.strip_prefix("*/").and_then(num).filter(|n| *n > 0);
    if (dom, month, dow) != ("*", "*", "*") {
        return cron.trim().to_string();
    }
    match (min, hour) {
        (m, "*") if every(m).is_some() => format!("every {} min", every(m).unwrap_or(1)),
        ("0", "*") => "hourly".into(),
        (m, h) if every(h).is_some() && num(m).is_some() => {
            format!("every {} h", every(h).unwrap_or(1))
        }
        (m, h) => match (num(m), num(h)) {
            (Some(m), Some(h)) if m < 60 && h < 24 => format!("daily at {h:02}:{m:02}"),
            _ => cron.trim().to_string(),
        },
    }
}

impl Tracker {
    /// A tracker that goes on from what the last read ended with.
    pub fn resume(state: Option<&Value>) -> Tracker {
        Tracker {
            state: state
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
        }
    }

    /// The tasks and loops after the lines read so far.
    pub fn signals(&self) -> Signals {
        Signals {
            tasks: self.state.tasks.clone(),
            loops: self.state.loops.iter().map(|(_, l)| l.clone()).collect(),
        }
    }

    /// What to hand to the next read; None when there is nothing to remember.
    pub fn state(mut self) -> Option<Value> {
        if self.state.calls.is_empty() && self.state.tasks.is_empty() && self.state.loops.is_empty()
        {
            return None;
        }
        let skip = self.state.calls.len().saturating_sub(CALLS_CAP);
        self.state.calls.drain(..skip);
        serde_json::to_value(&self.state).ok()
    }

    /// Take in one line of type `ty`.
    pub fn line(&mut self, v: &Value, ty: &str) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let ts = s(v, "timestamp").and_then(iso_to_ms).unwrap_or(0);
        match ty {
            "queue-operation" if s(v, "operation") == Some("enqueue") => {
                self.notice(s(v, "content").unwrap_or(""));
            }
            "assistant" => {
                let blocks = v.pointer("/message/content").and_then(Value::as_array);
                for b in blocks.into_iter().flatten() {
                    if s(b, "type") == Some("tool_use") {
                        self.tool_use(b, ts);
                    }
                }
            }
            "user" => self.user(v, ts),
            _ => {}
        }
    }

    fn user(&mut self, v: &Value, ts: i64) {
        match v.pointer("/message/content") {
            Some(Value::String(t)) => self.notice(t),
            Some(Value::Array(blocks)) => {
                let results = blocks
                    .iter()
                    .filter(|b| s(b, "type") == Some("tool_result"))
                    .count();
                for b in blocks {
                    match s(b, "type") {
                        Some("tool_result") => {
                            let own = (results == 1).then(|| v.get("toolUseResult")).flatten();
                            self.tool_result(b, own, ts);
                        }
                        Some("text") => self.notice(s(b, "text").unwrap_or("")),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn tool_use(&mut self, b: &Value, ts: i64) {
        let (Some(name), Some(id)) = (s(b, "name"), s(b, "id")) else {
            return;
        };
        let input = b.get("input").unwrap_or(&Value::Null);
        let label = |k: &str| s(input, k).map(one_line).filter(|l| !l.is_empty());
        let call = match name {
            // the call's own description says what it is for; the command line is the fallback
            "Bash" => Some(Call::Task {
                kind: "shell".into(),
                label: label("description")
                    .or_else(|| {
                        s(input, "command")
                            .map(command_label::of)
                            .filter(|l| !l.is_empty())
                    })
                    .unwrap_or_else(|| "shell".into()),
            }),
            "Agent" | "Task" => Some(Call::Task {
                kind: "agent".into(),
                label: label("description")
                    .or_else(|| label("prompt"))
                    .unwrap_or_else(|| "agent".into()),
            }),
            "Monitor" => Some(Call::Task {
                kind: "monitor".into(),
                label: label("description").unwrap_or_else(|| "monitor".into()),
            }),
            "CronCreate" => (input.get("recurring").and_then(Value::as_bool) != Some(false))
                .then(|| s(input, "cron"))
                .flatten()
                .map(|c| Call::Cron {
                    cron: c.trim().to_string(),
                    prompt: label("prompt").unwrap_or_default(),
                }),
            "CronDelete" => {
                if let Some(key) = s(input, "id") {
                    self.state.loops.retain(|(k, _)| k != key);
                }
                None
            }
            "TaskStop" | "KillShell" => {
                for k in ["task_id", "shell_id", "bash_id"] {
                    if let Some(task) = s(input, k) {
                        self.state.tasks.retain(|t| t.id != task);
                    }
                }
                None
            }
            "ScheduleWakeup" => {
                self.state.loops.retain(|(k, _)| k != WAKE_KEY);
                let delay = input.get("delaySeconds").and_then(Value::as_i64);
                if let (false, Some(delay)) = (input.get("stop") == Some(&Value::Bool(true)), delay)
                {
                    self.add_loop(
                        WAKE_KEY.into(),
                        LoopInfo {
                            kind: "self_paced".into(),
                            schedule: String::new(),
                            next_at: Some(ts + delay.clamp(0, 86_400) * 1000),
                            prompt: label("prompt").unwrap_or_default(),
                            started_at: ts,
                            cron: None,
                        },
                    );
                }
                None
            }
            _ => None,
        };
        if let Some(call) = call {
            self.state.calls.retain(|(k, _)| k != id);
            self.state.calls.push((id.to_string(), call));
        }
    }

    fn tool_result(&mut self, b: &Value, own: Option<&Value>, ts: i64) {
        let Some(id) = s(b, "tool_use_id") else {
            return;
        };
        let Some(at) = self.state.calls.iter().position(|(k, _)| k == id) else {
            return;
        };
        let (_, call) = self.state.calls.remove(at);
        if b.get("is_error").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let text = result_text(b.get("content"));
        let field = |k: &str| own.and_then(|o| s(o, k)).map(String::from);
        match call {
            Call::Task { kind, label } => {
                let task = field("backgroundTaskId")
                    .or_else(|| field("taskId"))
                    .or_else(|| {
                        field("agentId").filter(|_| own.is_some_and(|o| o["isAsync"] == true))
                    })
                    .or_else(|| {
                        let re = if kind == "agent" {
                            &RE_AGENT_ID
                        } else {
                            &RE_SHELL_ID
                        };
                        let async_agent = kind != "agent" || text.contains("Async agent launched");
                        re.captures(&text)
                            .filter(|_| async_agent)
                            .map(|c| c[1].to_string())
                    });
                if let Some(task) = task {
                    self.state.tasks.retain(|t| t.id != task);
                    self.state.tasks.push(BgTask {
                        id: task,
                        kind,
                        label,
                        started_at: ts,
                    });
                    let skip = self.state.tasks.len().saturating_sub(TASKS_CAP);
                    self.state.tasks.drain(..skip);
                }
            }
            Call::Cron { cron, prompt } => {
                let key = field("id")
                    .or_else(|| field("jobId"))
                    .unwrap_or_else(|| id.to_string());
                let every_n = cron.starts_with("*/") && cron.ends_with(" * * * *");
                self.add_loop(
                    key,
                    LoopInfo {
                        kind: "interval".into(),
                        schedule: describe_cron(&cron),
                        next_at: None,
                        prompt,
                        started_at: ts,
                        cron: every_n.then_some(cron),
                    },
                );
            }
        }
    }

    fn add_loop(&mut self, key: String, l: LoopInfo) {
        self.state.loops.retain(|(k, _)| *k != key);
        self.state.loops.push((key, l));
        let skip = self.state.loops.len().saturating_sub(LOOPS_CAP);
        self.state.loops.drain(..skip);
    }

    /// A `<task-notification>` in `text` ends the tasks it names, unless it says the task is
    /// still running.
    fn notice(&mut self, text: &str) {
        if !text.contains("<task-notification>") {
            return;
        }
        for block in text.split("<task-notification>").skip(1) {
            let running = RE_NOTICE_STATUS
                .captures(block)
                .is_some_and(|c| matches!(&c[1], "running" | "event" | "progress"));
            if running {
                continue;
            }
            if let Some(c) = RE_NOTICE_ID.captures(block) {
                let id = &c[1];
                self.state.tasks.retain(|t| t.id != id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn assistant(ts: &str, name: &str, id: &str, input: Value) -> Value {
        json!({"type": "assistant", "timestamp": ts, "message": {"role": "assistant",
            "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}})
    }

    fn result(ts: &str, id: &str, text: &str, own: Value) -> Value {
        json!({"type": "user", "timestamp": ts, "toolUseResult": own, "message": {"role": "user",
            "content": [{"type": "tool_result", "tool_use_id": id, "content": text}]}})
    }

    fn notice(ts: &str, id: &str, status: &str) -> Value {
        json!({"type": "queue-operation", "operation": "enqueue", "timestamp": ts,
            "content": format!("<task-notification>\n<task-id>{id}</task-id>\n<status>{status}</status>\n</task-notification>")})
    }

    fn feed(lines: &[Value]) -> Tracker {
        let mut t = Tracker::default();
        for v in lines {
            t.line(v, v["type"].as_str().unwrap());
        }
        t
    }

    const T1: &str = "2026-11-01T10:00:00.000Z";
    const T2: &str = "2026-11-01T10:05:00.000Z";

    #[test]
    fn a_background_shell_runs_until_its_notice() {
        let start = [
            assistant(
                T1,
                "Bash",
                "tu1",
                json!({"command": "npm  run\ndev", "run_in_background": true}),
            ),
            result(
                T1,
                "tu1",
                "Command running in background with ID: b1x",
                json!({"backgroundTaskId": "b1x"}),
            ),
        ];
        let mut t = feed(&start);
        let sig = t.signals();
        assert_eq!(sig.tasks.len(), 1);
        assert_eq!(
            (sig.tasks[0].kind.as_str(), sig.tasks[0].label.as_str()),
            ("shell", "npm run dev")
        );
        t.line(&notice(T2, "b1x", "completed"), "queue-operation");
        assert!(t.signals().tasks.is_empty());
    }

    #[test]
    fn a_background_shell_is_named_by_its_description_or_else_by_its_program() {
        let label_of = |input: Value| {
            let t = feed(&[
                assistant(T1, "Bash", "tu1", input),
                result(
                    T1,
                    "tu1",
                    "Command running in background with ID: b1x",
                    json!({"backgroundTaskId": "b1x"}),
                ),
            ]);
            t.signals().tasks[0].label.clone()
        };
        let setup = "S=/tmp/work/scratch; EYE_HOME=$S/x exec $S/bin daemon --port 7421";
        assert_eq!(
            label_of(json!({"command": setup, "description": "Run the daemon on port 7421"})),
            "Run the daemon on port 7421"
        );
        assert_eq!(
            label_of(json!({"command": setup, "description": "  "})),
            "bin daemon --port 7421"
        );
        assert_eq!(
            label_of(json!({"command": setup})),
            "bin daemon --port 7421"
        );
        assert_eq!(
            label_of(json!({"description": "Watch the build"})),
            "Watch the build"
        );
        assert_eq!(label_of(json!({})), "shell");
    }

    #[test]
    fn a_foreground_command_is_not_a_task_until_the_timeout_moves_it() {
        let t = feed(&[
            assistant(T1, "Bash", "tu1", json!({"command": "ls"})),
            result(T1, "tu1", "a b", json!({"stdout": "a b"})),
            assistant(T1, "Bash", "tu2", json!({"command": "make all"})),
            result(
                T1,
                "tu2",
                "moved",
                json!({"backgroundTaskId": "bz", "timedOutAfterMs": 120000}),
            ),
        ]);
        let ids: Vec<_> = t.signals().tasks.iter().map(|x| x.id.clone()).collect();
        assert_eq!(ids, ["bz"]);
    }

    #[test]
    fn a_background_agent_is_found_by_its_agent_id() {
        let t = feed(&[
            assistant(
                T1,
                "Agent",
                "tu1",
                json!({"description": "Build the docs", "run_in_background": true}),
            ),
            result(
                T1,
                "tu1",
                "Async agent launched successfully.\nagentId: a6c53112",
                json!({"isAsync": true, "agentId": "a6c53112"}),
            ),
        ]);
        let sig = t.signals();
        assert_eq!(
            (sig.tasks[0].kind.as_str(), sig.tasks[0].id.as_str()),
            ("agent", "a6c53112")
        );
        assert_eq!(sig.tasks[0].label, "Build the docs");
    }

    #[test]
    fn a_result_without_the_structured_part_is_read_from_its_text() {
        let t = feed(&[
            assistant(
                T1,
                "Bash",
                "tu1",
                json!({"command": "sleep 99", "run_in_background": true}),
            ),
            result(
                T1,
                "tu1",
                "Command running in background with ID: bq1. Output is ...",
                Value::Null,
            ),
            assistant(T1, "Agent", "tu2", json!({"description": "x"})),
            result(
                T1,
                "tu2",
                "agentId: a1 but this one ran in the foreground",
                Value::Null,
            ),
        ]);
        let ids: Vec<_> = t.signals().tasks.iter().map(|x| x.id.clone()).collect();
        assert_eq!(ids, ["bq1"]);
    }

    #[test]
    fn a_failed_launch_and_a_task_stop_leave_nothing() {
        let mut failed = result(T1, "tu1", "boom", json!({"backgroundTaskId": "b9"}));
        failed["message"]["content"][0]["is_error"] = json!(true);
        let t = feed(&[
            assistant(
                T1,
                "Bash",
                "tu1",
                json!({"command": "x", "run_in_background": true}),
            ),
            failed,
        ]);
        assert!(t.signals().tasks.is_empty());
        let t = feed(&[
            assistant(
                T1,
                "Bash",
                "tu1",
                json!({"command": "x", "run_in_background": true}),
            ),
            result(T1, "tu1", "ok", json!({"backgroundTaskId": "b2"})),
            assistant(T2, "TaskStop", "tu2", json!({"task_id": "b2"})),
        ]);
        assert!(t.signals().tasks.is_empty());
    }

    #[test]
    fn a_notice_that_says_running_does_not_end_a_monitor() {
        let mut t = feed(&[
            assistant(
                T1,
                "Monitor",
                "tu1",
                json!({"description": "errors in deploy.log", "timeout_ms": 60000}),
            ),
            result(T1, "tu1", "Monitor started", json!({"taskId": "m1"})),
        ]);
        assert_eq!(t.signals().tasks[0].kind, "monitor");
        t.line(&notice(T2, "m1", "running"), "queue-operation");
        assert_eq!(t.signals().tasks.len(), 1);
        t.line(&notice(T2, "m1", "killed"), "queue-operation");
        assert!(t.signals().tasks.is_empty());
    }

    #[test]
    fn a_notice_in_a_user_message_ends_the_task_too() {
        let mut t = feed(&[
            assistant(
                T1,
                "Bash",
                "tu1",
                json!({"command": "x", "run_in_background": true}),
            ),
            result(T1, "tu1", "ok", json!({"backgroundTaskId": "b3"})),
        ]);
        let msg = json!({"type": "user", "timestamp": T2, "message": {"role": "user",
            "content": "<task-notification>\n<task-id>b3</task-id>\n<status>failed</status>\n</task-notification>"}});
        t.line(&msg, "user");
        assert!(t.signals().tasks.is_empty());
    }

    #[test]
    fn a_cron_loop_lasts_until_it_is_deleted() {
        let mut t = feed(&[
            assistant(
                T1,
                "CronCreate",
                "tu1",
                json!({"cron": "*/10 * * * *", "prompt": "check the build", "recurring": true}),
            ),
            result(T1, "tu1", "created", json!({"id": "job1"})),
            assistant(
                T1,
                "CronCreate",
                "tu2",
                json!({"cron": "0 9 * * *", "prompt": "morning", "recurring": true}),
            ),
            result(T1, "tu2", "created", json!({"id": "job2"})),
            assistant(
                T1,
                "CronCreate",
                "tu3",
                json!({"cron": "5 * * * *", "prompt": "once", "recurring": false}),
            ),
            result(T1, "tu3", "created", json!({"id": "job3"})),
        ]);
        let loops = t.signals().loops;
        assert_eq!(loops.len(), 2);
        assert_eq!(
            (loops[0].schedule.as_str(), loops[0].cron.as_deref()),
            ("every 10 min", Some("*/10 * * * *"))
        );
        assert_eq!(
            (loops[1].schedule.as_str(), loops[1].cron.as_deref()),
            ("daily at 09:00", None)
        );
        t.line(
            &assistant(T2, "CronDelete", "tu4", json!({"id": "job1"})),
            "assistant",
        );
        assert_eq!(t.signals().loops.len(), 1);
    }

    #[test]
    fn a_wakeup_is_replaced_by_the_next_one_and_ends_with_stop() {
        let mut t = feed(&[assistant(
            T1,
            "ScheduleWakeup",
            "tu1",
            json!({"delaySeconds": 600, "prompt": "check", "reason": "r"}),
        )]);
        let first = t.signals().loops[0].next_at.unwrap();
        t.line(
            &assistant(
                T2,
                "ScheduleWakeup",
                "tu2",
                json!({"delaySeconds": 1200, "prompt": "check"}),
            ),
            "assistant",
        );
        let loops = t.signals().loops;
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].next_at.unwrap() - first, 5 * 60_000 + 600_000);
        assert_eq!(loops[0].kind, "self_paced");
        t.line(
            &assistant(T2, "ScheduleWakeup", "tu3", json!({"stop": true})),
            "assistant",
        );
        assert!(t.signals().loops.is_empty());
    }

    #[test]
    fn state_goes_on_from_one_read_to_the_next() {
        let mut t = feed(&[assistant(
            T1,
            "Bash",
            "tu1",
            json!({"command": "make", "run_in_background": true}),
        )]);
        t.line(
            &result(T1, "tu1", "ok", json!({"backgroundTaskId": "b4"})),
            "user",
        );
        t.line(
            &assistant(
                T1,
                "Bash",
                "tu2",
                json!({"command": "later", "run_in_background": true}),
            ),
            "assistant",
        );
        let state = t.state();
        let mut next = Tracker::resume(state.as_ref());
        next.line(
            &result(T2, "tu2", "ok", json!({"backgroundTaskId": "b5"})),
            "user",
        );
        next.line(&notice(T2, "b4", "completed"), "queue-operation");
        let ids: Vec<_> = next.signals().tasks.iter().map(|x| x.id.clone()).collect();
        assert_eq!(ids, ["b5"]);
        assert!(Tracker::resume(Some(&json!("junk"))).signals().is_empty());
    }

    #[test]
    fn sub_agent_lines_are_not_the_sessions_own() {
        let mut v = assistant(
            T1,
            "Bash",
            "tu1",
            json!({"command": "x", "run_in_background": true}),
        );
        v["isSidechain"] = json!(true);
        let mut r = result(T1, "tu1", "ok", json!({"backgroundTaskId": "b6"}));
        r["isSidechain"] = json!(true);
        assert!(feed(&[v, r]).signals().is_empty());
    }

    #[test]
    fn cron_words() {
        assert_eq!(describe_cron("*/5 * * * *"), "every 5 min");
        assert_eq!(describe_cron("0 * * * *"), "hourly");
        assert_eq!(describe_cron("15 */2 * * *"), "every 2 h");
        assert_eq!(describe_cron("30 8 * * *"), "daily at 08:30");
        assert_eq!(describe_cron("0 9 * * 1"), "0 9 * * 1");
        assert_eq!(describe_cron("nonsense"), "nonsense");
    }
}
