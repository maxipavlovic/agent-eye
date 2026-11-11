//! Background shells in a Codex rollout. A command that outlives its `exec_command` call keeps a
//! numeric session id, and the rollout records no start for it: the only trace is a tool result
//! with that `session_id` and no `exit_code` (or the older `Process running with session ID N`
//! header). It ends with a `CommandExecution` item for the same `process_id`, or a later result
//! for the id that carries an `exit_code`. The id is only visible when the program printed the
//! whole result, so a process whose id was never shown is not found.

use super::script::{self, Call};
use crate::agents::command_label;
use crate::agents::{BgTask, Signals};
use crate::util::iso_to_ms;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

const CALLS_CAP: usize = 64;
const TASKS_CAP: usize = 32;
/// What a shell is called when the call that started it does not show its command.
const UNKNOWN: &str = "background shell";

static RE_RUNNING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Process running with session ID (\d+)").expect("static regex"));

/// What a read hands to the next one.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// The commands of calls whose result has not been read, by call id.
    #[serde(default)]
    calls: Vec<(String, String)>,
    #[serde(default)]
    tasks: Vec<BgTask>,
}

/// Follows the lines of one rollout.
#[derive(Default)]
pub struct Tracker {
    state: State,
}

/// The texts of a tool output: a string or the text blocks of a list.
fn texts(output: Option<&Value>) -> Vec<&str> {
    match output {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect(),
        _ => vec![],
    }
}

/// An id Codex writes as a number or as a string.
fn id_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
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

    /// The background shells after the lines read so far (no loops: Codex has none in a rollout).
    pub fn signals(&self) -> Signals {
        Signals {
            tasks: self.state.tasks.clone(),
            loops: vec![],
        }
    }

    /// What to hand to the next read; None when there is nothing to remember.
    pub fn state(mut self) -> Option<Value> {
        if self.state.calls.is_empty() && self.state.tasks.is_empty() {
            return None;
        }
        let skip = self.state.calls.len().saturating_sub(CALLS_CAP);
        self.state.calls.drain(..skip);
        serde_json::to_value(&self.state).ok()
    }

    /// Take in one line of a rollout, as parsed.
    pub fn line(&mut self, v: &Value) {
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms)
            .unwrap_or(0);
        let Some(p) = v.get("payload") else {
            return;
        };
        let kind = p.get("type").and_then(Value::as_str);
        match (v.get("type").and_then(Value::as_str), kind) {
            (Some("response_item"), Some("custom_tool_call" | "function_call")) => self.call(p),
            (Some("response_item"), Some("custom_tool_call_output" | "function_call_output")) => {
                self.output(p, ts)
            }
            (Some("event_msg"), Some("item_completed")) => {
                let item = p.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("CommandExecution") {
                    self.end(id_of(item.get("process_id")));
                }
            }
            _ => {}
        }
    }

    /// A call that may start a shell: remember its command by call id.
    fn call(&mut self, p: &Value) {
        let Some(call_id) = p.get("call_id").and_then(Value::as_str) else {
            return;
        };
        let command = match p.get("name").and_then(Value::as_str) {
            Some("exec") => p.get("input").and_then(Value::as_str).and_then(|js| {
                script::calls(js).into_iter().find_map(|c| match c {
                    Call::Command(cmd) => Some(cmd),
                    Call::Patch(_) => None,
                })
            }),
            Some("exec_command") => p
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|a| serde_json::from_str::<Value>(a).ok())
                .and_then(|a| a.get("cmd").and_then(Value::as_str).map(String::from)),
            _ => return,
        };
        self.state.calls.retain(|(k, _)| k != call_id);
        self.state.calls.push((
            call_id.to_string(),
            command.map(|c| command_label::of(&c)).unwrap_or_default(),
        ));
    }

    /// A result: a shell still running, or the end of one.
    fn output(&mut self, p: &Value, ts: i64) {
        let call_id = p.get("call_id").and_then(Value::as_str).unwrap_or("");
        let label = self
            .state
            .calls
            .iter()
            .position(|(k, _)| k == call_id)
            .map(|at| self.state.calls.remove(at).1)
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| UNKNOWN.to_string());
        for text in texts(p.get("output")) {
            let json = serde_json::from_str::<Value>(text.trim()).ok();
            let id = json.as_ref().and_then(|j| id_of(j.get("session_id")));
            match (json, id) {
                (Some(j), Some(id)) if j.get("exit_code").is_some_and(|c| !c.is_null()) => {
                    self.end(Some(id))
                }
                (Some(_), Some(id)) => self.start(id, &label, ts),
                _ => {
                    if let Some(c) = RE_RUNNING.captures(text) {
                        self.start(c[1].to_string(), &label, ts);
                    }
                }
            }
        }
    }

    fn start(&mut self, id: String, label: &str, ts: i64) {
        if self.state.tasks.iter().any(|t| t.id == id) {
            return;
        }
        self.state.tasks.push(BgTask {
            id,
            kind: "shell".into(),
            label: label.to_string(),
            started_at: ts,
        });
        let skip = self.state.tasks.len().saturating_sub(TASKS_CAP);
        self.state.tasks.drain(..skip);
    }

    fn end(&mut self, id: Option<String>) {
        if let Some(id) = id {
            self.state.tasks.retain(|t| t.id != id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed(lines: &str) -> Tracker {
        let mut t = Tracker::default();
        for l in lines.lines() {
            t.line(&serde_json::from_str(l).unwrap());
        }
        t
    }

    const FIXTURE: &str = include_str!("../../../tests/fixtures/codex-background-process.jsonl");

    #[test]
    fn the_recorded_run_ends_with_its_process_closed_at_shutdown() {
        let lines: Vec<&str> = FIXTURE.lines().collect();
        let cut = |n: usize| lines[..n].join("\n");
        // after the first result: `sleep 80` runs, named by the command the call showed
        let t = feed(&cut(5));
        let tasks = t.signals().tasks;
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            (
                tasks[0].id.as_str(),
                tasks[0].kind.as_str(),
                tasks[0].label.as_str()
            ),
            ("36096", "shell", "sleep 80")
        );
        // the foreground `sleep 25` with its exit code never counts
        assert_eq!(feed(&cut(9)).signals().tasks.len(), 1);
        // the closing item of process 36096 ends it
        assert!(feed(FIXTURE).signals().tasks.is_empty());
    }

    #[test]
    fn a_shell_is_named_by_its_program_not_by_its_set_up() {
        let mut t = Tracker::default();
        let cmd = "S=/tmp/work/scratch; EYE_HOME=$S/x exec $S/bin daemon --port 7421";
        t.line(&json!({"type":"response_item","payload":{"type":"function_call","call_id":"c1","name":"exec_command",
            "arguments": json!({"cmd": cmd}).to_string()}}));
        t.line(
            &json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"c1",
            "output":"Chunk ID: 1\nProcess running with session ID 7\nOutput:\n"}}),
        );
        assert_eq!(t.signals().tasks[0].label, "bin daemon --port 7421");
    }

    #[test]
    fn a_result_that_names_no_command_gets_a_plain_label() {
        let t = feed(
            r#"{"timestamp":"2026-11-11T20:00:34.148Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"x","output":[{"type":"input_text","text":"{\"session_id\":7,\"output\":\"\"}"}]}}"#,
        );
        assert_eq!(t.signals().tasks[0].label, "background shell");
    }

    #[test]
    fn the_older_header_starts_a_shell_and_a_later_exit_code_ends_it() {
        let mut t = Tracker::default();
        let call = json!({"type":"response_item","timestamp":"2026-11-11T20:00:00Z","payload":{"type":"function_call","call_id":"c1","name":"exec_command","arguments":"{\"cmd\":\"npm run dev\"}"}});
        let out = |text: &str| json!({"type":"response_item","timestamp":"2026-11-11T20:00:01Z","payload":{"type":"function_call_output","call_id":"c2","output":text}});
        t.line(&call);
        t.line(&json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"Chunk ID: 1\nProcess running with session ID 4242\nOutput:\n"}}));
        let tasks = t.signals().tasks;
        assert_eq!(
            (tasks[0].id.as_str(), tasks[0].label.as_str()),
            ("4242", "npm run dev")
        );
        t.line(&out(
            "{\"session_id\":4242,\"exit_code\":0,\"output\":\"\"}",
        ));
        assert!(t.signals().tasks.is_empty());
    }

    #[test]
    fn a_started_shell_is_listed_once_and_state_goes_on_between_reads() {
        let mut t = Tracker::default();
        let out = json!({"type":"response_item","timestamp":"2026-11-11T20:00:01Z","payload":{"type":"custom_tool_call_output","call_id":"c","output":[{"type":"input_text","text":"{\"session_id\":9}"}]}});
        t.line(&out);
        t.line(&out);
        assert_eq!(t.signals().tasks.len(), 1);
        let state = t.state();
        let mut next = Tracker::resume(state.as_ref());
        assert_eq!(next.signals().tasks.len(), 1);
        next.line(&json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"CommandExecution","process_id":"9","exit_code":0}}}));
        assert!(next.signals().is_empty());
        assert!(Tracker::resume(Some(&json!(3))).signals().is_empty());
        assert_eq!(Tracker::default().state(), None);
    }

    #[test]
    fn other_lines_and_other_json_are_ignored() {
        let t = feed(
            "{\"type\":\"session_meta\",\"payload\":{}}\n\
             {\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"c\",\"output\":\"{\\\"chunk_id\\\":\\\"a\\\",\\\"exit_code\\\":0}\"}}\n\
             {\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"c\",\"output\":\"not json\"}}",
        );
        assert!(t.signals().is_empty());
    }
}
