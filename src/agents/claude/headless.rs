//! One-shot questions to Claude Code without a session: `claude -p` with structured output. Runs
//! from an empty folder, keeps no transcript, loads no tools, MCP servers or hooks.

use super::Claude;
use crate::agents::{HeadlessReply, HeadlessRequest};
use crate::util::head_chars;
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{BufReader, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Read what the `claude -p --output-format json` run printed: the structured answer and the
/// numbers around it.
pub fn parse_reply(raw: &str) -> Result<HeadlessReply> {
    let v: Value = serde_json::from_str(raw.trim()).context("claude output is not JSON")?;
    if v.get("is_error").and_then(Value::as_bool) == Some(true) {
        anyhow::bail!(
            "claude reported an error: {}",
            head_chars(v.get("result").and_then(Value::as_str).unwrap_or("?"), 300)
        );
    }
    let output = v
        .get("structured_output")
        .context("no structured_output in result")?
        .clone();
    Ok(HeadlessReply {
        output,
        duration_ms: v.get("duration_ms").and_then(Value::as_i64).unwrap_or(0),
        usage: v.get("usage").cloned().unwrap_or(Value::Null),
        cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
    })
}

fn drain<R: Read + Send + 'static>(r: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = BufReader::new(r).read_to_end(&mut v);
        v
    })
}

/// Run `claude -p` on a request and return what it printed.
fn run(c: &Claude, req: &HeadlessRequest) -> Result<String> {
    let mut child = Command::new(c.executable_path())
        .args([
            "-p",
            "--model",
            req.model,
            "--no-session-persistence",
            "--tools",
            "",
            "--strict-mcp-config",
            "--settings",
            r#"{"disableAllHooks":true,"autoMemoryEnabled":false}"#,
            "--output-format",
            "json",
            "--json-schema",
            &req.schema.to_string(),
            "--system-prompt",
            req.system,
            req.input,
        ])
        .current_dir(req.work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting `claude` (is it on PATH?)")?;
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() > req.timeout {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("claude timed out after {}s", req.timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = out
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let err = err
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    if !status.success() && out.is_empty() {
        anyhow::bail!(
            "claude exited with {status}: {}",
            head_chars(&String::from_utf8_lossy(&err), 300)
        );
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Ask Claude Code one structured question and read the answer.
pub fn ask(c: &Claude, req: &HeadlessRequest) -> Result<HeadlessReply> {
    parse_reply(&run(c, req)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_recorded_envelope() {
        let raw = r#"{"type":"result","is_error":false,"duration_ms":3210,"total_cost_usd":0.0123,
          "usage":{"input_tokens":10,"output_tokens":5},
          "structured_output":{"status":"done"}}"#;
        let r = parse_reply(raw).unwrap();
        assert_eq!(r.output["status"], "done");
        assert_eq!(r.duration_ms, 3210);
        assert_eq!(r.cost_usd, Some(0.0123));
        assert_eq!(r.usage["output_tokens"], 5);
        assert!(parse_reply("not json").is_err());
        assert!(parse_reply(r#"{"is_error":true,"result":"limit reached"}"#)
            .unwrap_err()
            .to_string()
            .contains("limit reached"));
        assert!(parse_reply(r#"{"type":"result"}"#).is_err());
        let free = parse_reply(r#"{"type":"result","structured_output":{}}"#).unwrap();
        assert_eq!(free.cost_usd, None, "no cost said is no cost, not zero");
    }
}
