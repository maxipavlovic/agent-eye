//! Summaries through `agy -p`: print mode with `--output-format json` and `--json-schema`, which
//! answers with one JSON object (`status`, `response`, `structured_output`, `usage`). agy has no
//! flag to leave no conversation behind, so the run happens in a folder named `eye-ask-<uuid>`,
//! and the scanner leaves conversations in such folders out.

use super::files::ASK_PREFIX;
use super::Agy;
use crate::agents::{HeadlessReply, HeadlessRequest};
use crate::util::{head_chars, new_uuid};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The JSON in a reply that may be wrapped in a code fence.
fn fenced_json(text: &str) -> Result<Value> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map_or(t, |r| r.trim_end_matches("```").trim());
    serde_json::from_str(t).with_context(|| {
        format!(
            "agy did not answer with JSON: {}",
            head_chars(text.trim(), 200)
        )
    })
}

/// What `agy -p --output-format json` printed, as the reply.
pub fn reply_of(stdout: &str, duration_ms: i64) -> Result<HeadlessReply> {
    let v: Value = serde_json::from_str(stdout.trim())
        .with_context(|| format!("agy printed no JSON: {}", head_chars(stdout.trim(), 200)))?;
    let status = v.get("status").and_then(Value::as_str).unwrap_or("");
    anyhow::ensure!(
        status == "SUCCESS",
        "agy answered with status {status:?}: {}",
        head_chars(&v.to_string(), 300)
    );
    let output = match v.get("structured_output").filter(|o| o.is_object()) {
        Some(o) => o.clone(),
        None => fenced_json(v.get("response").and_then(Value::as_str).unwrap_or(""))?,
    };
    let u = v.get("usage").cloned().unwrap_or(Value::Null);
    let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
    Ok(HeadlessReply {
        output,
        duration_ms,
        usage: json!({
            "input_tokens": n("input_tokens"),
            "output_tokens": n("output_tokens"),
            "cache_read_input_tokens": n("cache_read_tokens"),
        }),
        cost_usd: None,
    })
}

/// The arguments of a summary run.
fn command_args(prompt: String, schema: &Value, model: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["-p".into(), prompt];
    args.extend(
        [
            "--output-format",
            "json",
            "--disable-slash-commands",
            // a summary needs no tools: plan mode does not edit or run commands, and the sandbox
            // confines whatever still runs (agy's changelog: `--mode plan` applies in `-p` runs;
            // `--sandbox` is a flag without a value). No `--print-timeout`: its unit is not
            // documented and `--help` was not run here; the loop below kills agy at `req.timeout`.
            "--mode",
            "plan",
            "--sandbox",
            "--json-schema",
        ]
        .map(String::from),
    );
    args.push(schema.to_string());
    if !model.is_empty() {
        args.extend(["--model".into(), model.into()]);
    }
    args
}

pub fn ask(a: &Agy, req: &HeadlessRequest) -> Result<HeadlessReply> {
    std::fs::create_dir_all(req.work_dir)?;
    let dir = req.work_dir.join(format!("{ASK_PREFIX}{}", new_uuid()?));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let scratch = Scratch(dir);
    let mut prompt = format!("{}\n\n{}", req.system.trim_end(), req.input);
    if prompt.starts_with('-') {
        prompt.insert_str(0, "Task: ");
    }
    let args = command_args(prompt, req.schema, req.model);
    let exe = a.executable_path();
    let started = Instant::now();
    let mut child = Command::new(&exe)
        .args(&args)
        .current_dir(&scratch.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting `{exe}` (is it on PATH?)"))?;
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut v = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut v);
            }
            v
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as _));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as _));
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if started.elapsed() > req.timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("agy timed out after {}s", req.timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    anyhow::ensure!(
        status.success(),
        "agy exited with {status}: {}",
        head_chars(err.trim(), 300)
    );
    reply_of(&out, started.elapsed().as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_that_stop_tools_are_in_the_command() {
        let args = command_args("p".into(), &json!({"type":"object"}), "m");
        let at = |f: &str| args.iter().position(|a| a == f).unwrap();
        assert_eq!(args[at("--mode") + 1], "plan");
        assert!(args.contains(&"--sandbox".to_string()));
        assert_eq!(args[at("--model") + 1], "m");
        // the flag value of --json-schema is the next argument, not --sandbox
        assert_eq!(args[at("--json-schema") + 1], "{\"type\":\"object\"}");
    }

    #[test]
    fn the_structured_output_is_the_answer() {
        let out = json!({"conversation_id":"c","status":"SUCCESS","response":"{\"a\":1}\n",
            "structured_output":{"a":1},"usage":{"input_tokens":9,"output_tokens":2,"cache_read_tokens":4}});
        let r = reply_of(&out.to_string(), 5).unwrap();
        assert_eq!(r.output["a"], 1);
        assert_eq!(r.usage["input_tokens"], 9);
        assert_eq!(r.usage["cache_read_input_tokens"], 4);
        assert!(r.cost_usd.is_none());
    }

    #[test]
    fn a_fenced_response_is_read_and_a_failure_is_an_error() {
        let out = json!({"status":"SUCCESS","response":"```json\n{\"b\":2}\n```"});
        assert_eq!(reply_of(&out.to_string(), 1).unwrap().output["b"], 2);
        let bad = json!({"status":"ERROR","response":""});
        assert!(reply_of(&bad.to_string(), 1)
            .unwrap_err()
            .to_string()
            .contains("status"));
        assert!(reply_of("not json", 1).is_err());
        let prose = json!({"status":"SUCCESS","response":"Sorry."});
        assert!(reply_of(&prose.to_string(), 1)
            .unwrap_err()
            .to_string()
            .contains("did not answer"));
    }
}
