//! Summaries through `opencode run --format json`. OpenCode cannot constrain an answer to a schema,
//! so the prompt asks for strict JSON, the last text of the event stream is validated against the
//! schema eye uses, and one invalid answer is asked again. The run is made `--archived` (out of
//! OpenCode's lists) and its session is deleted afterwards, also when the answer is invalid or the
//! run times out, so it never shows up in eye or in OpenCode. Every question runs in one folder
//! (`opencode-ask`) with a session title of its own. It runs as an agent of its own with
//! every permission denied and every tool off, a private server and the user's MCP servers switched
//! off (`lockdown.rs`), so text inside what it is asked about (a quoted web page, a tool's output)
//! has nothing to drive. Checked on a real OpenCode 2.3.2 (`docs/security.md`).

use super::{lockdown, OpenCode};
use crate::agents::{HeadlessReply, HeadlessRequest};
use crate::util::{head_chars, new_uuid};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The model name that means "the cheapest free model OpenCode lists".
pub const FREE: &str = "free";
/// Words in a model name that mark a small, fast one; the first free model with one is taken.
const SMALL: [&str; 6] = ["flash", "lightning", "small", "mini", "nano", "lite"];

/// The flags a summary run needs `opencode run` to have: without `--agent` the run would be the
/// user's default agent, and without `--standalone` OpenCode's background service, neither of
/// which keeps to the run's config.
const FLAGS: [&str; 5] = [
    "--format",
    "--archived",
    "--title",
    "--agent",
    "--standalone",
];

static CHECKED: LazyLock<Mutex<HashMap<String, Result<(), String>>>> =
    LazyLock::new(Mutex::default);
static FREE_MODEL: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Mutex::default);

/// Does `opencode run` know the flags a summary needs? Asked once per program, with `--help`.
pub fn flags_check(c: &OpenCode) -> Result<()> {
    let exe = c.executable_path();
    if let Some(r) = CHECKED.lock().unwrap_or_else(|e| e.into_inner()).get(&exe) {
        return r.clone().map_err(anyhow::Error::msg);
    }
    let out = c
        .command(&["run", "--help"])
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting `{exe}` (is it on PATH?)"))?;
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let missing: Vec<&str> = FLAGS.into_iter().filter(|f| !help.contains(f)).collect();
    let r = if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "this OpenCode has no {} for `opencode run`: update OpenCode, or let another agent write the summaries ([summary] agent in config.toml)",
            missing.join(", ")
        ))
    };
    CHECKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(exe, r.clone());
    r.map_err(anyhow::Error::msg)
}

/// The free model a summary uses when none is named: of the `provider/model` lines `opencode
/// models` prints, those ending in `-free`, a small one first. None when there is none (OpenCode
/// then uses its own default model).
pub fn pick_free(models: &str) -> Option<String> {
    let free: Vec<&str> = models
        .lines()
        .map(str::trim)
        .filter(|m| m.ends_with("-free") && m.contains('/'))
        .collect();
    free.iter()
        .find(|m| SMALL.iter().any(|s| m.contains(s)))
        .or(free.first())
        .map(|m| m.to_string())
}

/// The free model, asked in `dir` (the questions' folder, so OpenCode makes no project elsewhere).
fn free_model(c: &OpenCode, dir: &Path) -> Option<String> {
    let exe = c.executable_path();
    if let Some(m) = FREE_MODEL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&exe)
    {
        return m.clone();
    }
    let out = c
        .command(&["models"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let m = pick_free(&String::from_utf8_lossy(&out.stdout));
    FREE_MODEL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(exe, m.clone());
    m
}

/// The folder every question runs in, under eye's work folder. OpenCode keeps a project for each
/// folder it starts in and has no command to remove one, so one folder for all questions leaves one
/// project. Made once (owner only) and never removed: another question may be running in it, and
/// nothing is written to it.
const ASK_DIR: &str = "opencode-ask";

/// The start of a question's session title; a uuid follows, so that a clean-up deletes the
/// question's own session and never one of another question running in the same folder.
pub(super) const ASK_TITLE: &str = "eye-ask-";

fn ask_dir(work: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(work).with_context(|| format!("creating {}", work.display()))?;
    let dir = work.join(ASK_DIR);
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(e).with_context(|| format!("creating {}", dir.display()))
        }
        _ => Ok(dir),
    }
}

/// What the runs of one question share: the title of their sessions and the MCP servers they
/// switch off.
struct Question {
    title: String,
    mcp: Vec<String>,
}

pub(super) fn drain<R: Read + Send + 'static>(r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = BufReader::new(r).read_to_end(&mut v);
        v
    })
}

/// What one `opencode run` printed: the session it made, the text of its last reply and the tokens
/// and cost it reports.
#[derive(Debug, Default, PartialEq)]
pub struct Run {
    pub session: Option<String>,
    pub text: String,
    pub usage: Value,
    pub cost: Option<f64>,
}

/// Read the event stream: one JSON object per line. The answer is the text of the last message
/// that has text (a model that calls tools first writes its answer in the last one).
pub fn read_events(out: &str) -> Run {
    let mut run = Run::default();
    let mut last_message: Option<String> = None;
    let (mut input, mut output, mut cache_read, mut cache_write, mut cost) =
        (0i64, 0i64, 0i64, 0i64, 0.0f64);
    let mut priced = false;
    for line in out.lines() {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(s) = e.get("sessionID").and_then(Value::as_str) {
            run.session.get_or_insert_with(|| s.to_string());
        }
        let part = e.get("part").unwrap_or(&Value::Null);
        match e.get("type").and_then(Value::as_str) {
            Some("text") => {
                let msg = part
                    .get("messageID")
                    .and_then(Value::as_str)
                    .map(String::from);
                let t = part.get("text").and_then(Value::as_str).unwrap_or("");
                if msg != last_message {
                    run.text.clear();
                    last_message = msg;
                }
                run.text.push_str(t);
            }
            Some("step_finish") => {
                let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
                if let Some(t) = part.get("tokens") {
                    input += n(t, "input");
                    output += n(t, "output") + n(t, "reasoning");
                    let c = t.get("cache").unwrap_or(&Value::Null);
                    cache_read += n(c, "read");
                    cache_write += n(c, "write");
                }
                if let Some(c) = part.get("cost").and_then(Value::as_f64) {
                    cost += c;
                    priced = true;
                }
            }
            _ => {}
        }
    }
    run.usage = json!({"input_tokens": input, "output_tokens": output,
                       "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": cache_write});
    run.cost = priced.then_some(cost);
    run
}

/// The JSON in a reply, which may be wrapped in a code fence or have a sentence around it.
pub fn json_in(text: &str) -> Result<Value> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |r| r.trim_end_matches("```").trim());
    if let Ok(v) = serde_json::from_str(t) {
        return Ok(v);
    }
    // a sentence around it: the outermost braces
    if let (Some(a), Some(b)) = (t.find('{'), t.rfind('}')) {
        if a < b {
            if let Ok(v) = serde_json::from_str(&t[a..=b]) {
                return Ok(v);
            }
        }
    }
    anyhow::bail!("not JSON: {}", head_chars(text.trim(), 200))
}

/// Check `v` against the part of JSON schema eye's schemas use: `type`, `enum`, `required`,
/// `properties` and `items`. The first problem is the error.
pub fn validate(schema: &Value, v: &Value, at: &str) -> Result<(), String> {
    if let Some(t) = schema.get("type").and_then(Value::as_str) {
        let ok = match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            "boolean" => v.is_boolean(),
            _ => true,
        };
        if !ok {
            return Err(format!("{at} must be {t}"));
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            return Err(format!("{at} must be one of {}", Value::Array(e.clone())));
        }
    }
    if let (Some(props), Some(obj)) = (
        schema.get("properties").and_then(Value::as_object),
        v.as_object(),
    ) {
        for r in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !obj.contains_key(r) {
                return Err(format!("{at}.{r} is missing"));
            }
        }
        for (k, ps) in props {
            if let Some(x) = obj.get(k) {
                validate(ps, x, &format!("{at}.{k}"))?;
            }
        }
    }
    if let (Some(items), Some(arr)) = (schema.get("items"), v.as_array()) {
        for (i, x) in arr.iter().enumerate() {
            validate(items, x, &format!("{at}[{i}]"))?;
        }
    }
    Ok(())
}

/// The prompt: the instructions, the question and the schema, with a demand for bare JSON.
fn prompt(req: &HeadlessRequest, problem: Option<&str>) -> String {
    let mut p = format!(
        "{}\n\n{}\n\nAnswer with one JSON object that follows this JSON schema, and nothing else: no code fence, no explanation, no tool calls.\n{}",
        req.system.trim_end(),
        req.input,
        req.schema
    );
    if let Some(why) = problem {
        p.push_str(&format!(
            "\n\nYour previous answer was not valid ({why}). Answer again with only the JSON object."
        ));
    }
    p
}

/// The command of one question: `opencode run` in its own archived session, as the locked-down
/// agent, with the MCP servers the question names switched off.
fn ask_command(c: &OpenCode, model: &str, q: &Question) -> Command {
    let mut cmd = c.command(&[
        "run",
        "--format",
        "json",
        "--archived",
        "--title",
        &q.title,
        "--agent",
        lockdown::AGENT,
    ]);
    cmd.env(lockdown::CONFIG_VAR, lockdown::config(&q.mcp));
    if !model.is_empty() {
        cmd.args(["-m", model]);
    }
    cmd
}

/// Run `opencode run` once on `prompt`; its stdout. The session it made is deleted by the caller.
fn run_once(
    c: &OpenCode,
    model: &str,
    q: &Question,
    prompt: String,
    dir: &Path,
    timeout: Duration,
) -> Result<String> {
    let mut child = ask_command(c, model, q)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting `{}` (is it on PATH?)", c.executable_path()))?;
    let mut input = child.stdin.take().context("no stdin")?;
    let writer = std::thread::spawn(move || {
        let _ = input.write_all(prompt.as_bytes());
    });
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("opencode timed out after {}s", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = writer.join();
    let out = out
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let err = err
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    anyhow::ensure!(
        status.success(),
        "opencode exited with {status}: {}",
        head_chars(String::from_utf8_lossy(&err).trim(), 300)
    );
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Ask OpenCode one structured question. See the module comment.
pub fn ask(c: &OpenCode, req: &HeadlessRequest) -> Result<HeadlessReply> {
    flags_check(c)?;
    let dir = ask_dir(req.work_dir)?;
    let model = match req.model.trim() {
        "" | FREE => free_model(c, &dir).unwrap_or_default(),
        m => m.to_string(),
    };
    let started = Instant::now();
    let q = Question {
        title: format!("{ASK_TITLE}{}", new_uuid()?),
        mcp: c.mcp_servers(&dir),
    };
    let mut cost = None::<f64>;
    let mut problem: Option<String> = None;
    let mut last_err = anyhow::anyhow!("no answer");
    for _ in 0..2 {
        let left = req.timeout.saturating_sub(started.elapsed());
        let out = run_once(c, &model, &q, prompt(req, problem.as_deref()), &dir, left);
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                c.delete_ask_sessions(&dir, &q.title);
                return Err(e);
            }
        };
        let run = read_events(&out);
        if let Some(id) = &run.session {
            c.delete_session(id, &dir);
        }
        cost = run.cost.or(cost);
        match json_in(&run.text).and_then(|v| match validate(req.schema, &v, "$") {
            Ok(()) => Ok(v),
            Err(e) => Err(anyhow::anyhow!(e)),
        }) {
            Ok(output) => {
                return Ok(HeadlessReply {
                    output,
                    duration_ms: started.elapsed().as_millis() as i64,
                    usage: run.usage,
                    cost_usd: cost,
                })
            }
            Err(e) => {
                problem = Some(head_chars(&format!("{e:#}"), 200));
                last_err = e;
            }
        }
    }
    Err(last_err.context("opencode did not answer with JSON that follows the schema, twice"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(cmd: &Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    /// The whole lock-down of a question's command: a `*` deny rule globally and for its own agent,
    /// every tool off, the user's MCP servers off, no user config named, a private server, no update.
    #[test]
    fn a_question_runs_as_its_own_agent_with_everything_denied_and_the_mcp_servers_off() {
        let c = OpenCode::new(std::path::PathBuf::from("/nonexistent"));
        let q = Question {
            title: "eye-ask-1".into(),
            mcp: vec!["jira".into(), "docs".into()],
        };
        let cmd = ask_command(&c, "provider/model", &q);
        let rules: Value =
            serde_json::from_str(&env_of(&cmd, lockdown::CONFIG_VAR).unwrap()).unwrap();
        // the default rule, so what the list below leaves out (an MCP tool, code mode) is denied
        // too; the agent's own rules say it again, as OpenCode applies them after the global ones
        let agent = &rules["agent"][lockdown::AGENT];
        for at in [&rules["permission"], &agent["permission"]] {
            assert_eq!(at["*"], "deny");
            for tool in [
                "bash",
                "edit",
                "webfetch",
                "websearch",
                "external_directory",
                "task",
                "skill",
            ] {
                assert_eq!(at[tool], "deny", "{tool}");
            }
            assert!(at.as_object().unwrap().values().all(|v| v == "deny"));
        }
        assert_eq!(rules["tools"]["*"], false);
        assert_eq!(agent["tools"]["*"], false);
        assert_eq!(agent["mode"], "primary");
        for server in ["jira", "docs"] {
            assert_eq!(rules["mcp"][server]["enabled"], false, "{server}");
        }
        assert_eq!(rules["autoupdate"], false);
        assert_eq!(rules["share"], "disabled");
        // nothing in the config allows or asks anything, and the run is not told to approve what it asks
        assert!(!rules.to_string().contains("allow"));
        assert!(!rules.to_string().contains("\"ask\""));
        // inline only: the user's own config file is not named, written or read
        assert!(cmd.get_envs().all(|(k, _)| k != "OPENCODE_CONFIG"));
        for (k, v) in lockdown::ENV {
            assert_eq!(env_of(&cmd, k).as_deref(), Some(v), "{k}");
        }
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args
            .iter()
            .any(|a| a == "--auto" || a.contains("dangerously")));
        assert!(args.windows(2).any(|w| w == ["--agent", lockdown::AGENT]));
        assert!(args.windows(2).any(|w| w == ["-m", "provider/model"]));
        assert!(args.contains(&"--archived".to_string()));
        assert!(args.windows(2).any(|w| w == ["--title", "eye-ask-1"]));
        assert!(args.contains(&"--standalone".to_string()));
    }

    #[test]
    fn the_last_message_with_text_is_the_answer_and_usage_adds_up() {
        let out = [
            r#"{"type":"step_start","sessionID":"ses_1","part":{}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"messageID":"m1","text":"I will look."}}"#,
            r#"{"type":"step_finish","sessionID":"ses_1","part":{"tokens":{"input":10,"output":2,"reasoning":1,"cache":{"read":5,"write":0}},"cost":0.5}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"messageID":"m2","text":"{\"a\":"}}"#,
            r#"{"type":"text","sessionID":"ses_1","part":{"messageID":"m2","text":"1}"}}"#,
            r#"{"type":"step_finish","sessionID":"ses_1","part":{"tokens":{"input":20,"output":4,"reasoning":0,"cache":{"read":0,"write":3}},"cost":0}}"#,
            "not json",
        ]
        .join("\n");
        let r = read_events(&out);
        assert_eq!(
            (r.session.as_deref(), r.text.as_str(), r.cost),
            (Some("ses_1"), "{\"a\":1}", Some(0.5))
        );
        assert_eq!(r.usage["input_tokens"], 30);
        assert_eq!(r.usage["output_tokens"], 7);
        assert_eq!(r.usage["cache_creation_input_tokens"], 3);
    }

    #[test]
    fn the_event_streams_of_a_real_run_are_read() {
        let ask = read_events(include_str!(
            "../../../tests/fixtures/opencode-real/run-ask.jsonl"
        ));
        assert!(ask
            .session
            .as_deref()
            .is_some_and(|s| s.starts_with("ses_")));
        assert_eq!(json_in(&ask.text).unwrap()["a"], 1);
        assert_eq!(ask.cost, Some(0.0));
        assert!(ask.usage["input_tokens"].as_i64().unwrap() > 0);
        // a run that used a tool first: the answer is the last message's text
        let tool = read_events(include_str!(
            "../../../tests/fixtures/opencode-real/run-tool.jsonl"
        ));
        assert_eq!(tool.text.trim(), "done");
    }

    #[test]
    fn json_is_found_in_a_fence_or_a_sentence() {
        assert_eq!(json_in("```json\n{\"a\":1}\n```").unwrap()["a"], 1);
        assert_eq!(json_in("Sure! {\"a\":1} Hope it helps").unwrap()["a"], 1);
        assert!(json_in("no braces").is_err());
    }

    #[test]
    fn the_summary_schema_validates_and_says_what_is_wrong() {
        let schema = crate::summarize::schema();
        let ok = json!({"status": "done", "state": "s", "next": "n",
                        "topics": [{"key": "k", "title": "t", "status": "active", "next": "n"}]});
        assert_eq!(validate(&schema, &ok, "$"), Ok(()));
        let bad = |v: Value| validate(&schema, &v, "$").unwrap_err();
        assert_eq!(
            bad(json!({"status": "done", "state": "s", "next": "n"})),
            "$.topics is missing"
        );
        assert!(
            bad(json!({"status": "bogus", "state": "s", "next": "n", "topics": []}))
                .starts_with("$.status must be one of")
        );
        assert!(
            bad(json!({"status": "done", "state": 1, "next": "n", "topics": []}))
                .contains("$.state must be string")
        );
        assert!(bad(
            json!({"status": "done", "state": "s", "next": "n", "topics": [{"key": "k"}]})
        )
        .contains("$.topics[0].title is missing"));
        assert_eq!(bad(json!([1])), "$ must be object");
    }

    #[test]
    fn the_free_model_is_a_small_one_when_there_is() {
        let list = "anthropic/claude-sonnet-4-5\nopencode/big-free\nopencode/iceman-free\nopencode/x-flash-free\nopencode/paid";
        assert_eq!(pick_free(list).as_deref(), Some("opencode/x-flash-free"));
        assert_eq!(
            pick_free("opencode/big-free\nopencode/iceman-free").as_deref(),
            Some("opencode/big-free")
        );
        assert_eq!(pick_free("openai/gpt-5\n"), None);
    }
}
