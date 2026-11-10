//! `eye hook <Event> [--agent <id>]`: append one line to the queue and get out of the way. The
//! agent's adapter decides what of the payload is kept. It prints nothing and the caller always
//! exits 0, whatever happens.

use crate::agents::Agent;
use crate::paths::Paths;
use crate::util::now_ms;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;

/// A hook payload is read up to this many bytes.
pub const MAX_STDIN: u64 = 1 << 20;
/// The queue file the hooks append to and the daemon drains.
pub fn queue_path(paths: &Paths) -> std::path::PathBuf {
    paths.home.join("queue").join("events.jsonl")
}

fn log_error(paths: &Paths, msg: &str) {
    let dir = paths.home.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(dir.join("hook-errors.log"))
    {
        let _ = writeln!(f, "{} {}", now_ms(), msg);
    }
}

fn append(paths: &Paths, line: &Value) -> std::io::Result<()> {
    let p = queue_path(paths);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut text = line.to_string();
    text.push('\n');
    // One write() on an O_APPEND file: concurrent hooks never interleave a line.
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(text.as_bytes())
}

/// Never fails and never panics out of this function. The queue line carries the agent's id.
pub fn run(paths: &Paths, agent: &dyn Agent, event: &str, mut input: impl Read) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut buf = Vec::new();
        if let Err(e) = (&mut input).take(MAX_STDIN).read_to_end(&mut buf) {
            log_error(paths, &format!("{event}: reading stdin: {e}"));
            return;
        }
        let payload: Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(e) => {
                log_error(paths, &format!("{event}: bad stdin json: {e}"));
                return;
            }
        };
        let Some(mut line) = agent.queue_line(event, &payload, now_ms()) else {
            return;
        };
        if let Some(m) = line.as_object_mut() {
            m.insert("agent".into(), json!(agent.id()));
        }
        if let Err(e) = append(paths, &line) {
            log_error(paths, &format!("{event}: append: {e}"));
        }
    }));
    if r.is_err() {
        log_error(paths, &format!("{event}: panic"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::agents::claude::Claude;

    fn paths() -> (tempfile::TempDir, Paths) {
        let t = tempfile::tempdir().unwrap();
        let p = Paths {
            home: t.path().join("h"),
        };
        (t, p)
    }

    fn claude() -> Claude {
        Claude::new("/nonexistent/c".into())
    }

    #[test]
    fn appends_one_line_per_event() {
        let (_t, p) = paths();
        let payload = r#"{"session_id":"s1","transcript_path":"/x/s1.jsonl","cwd":"/x","hook_event_name":"Stop","prompt":"SECRET PROMPT","last_assistant_message":"Done. Shall I go on?","background_tasks":[1,2]}"#;
        run(&p, &claude(), "Stop", payload.as_bytes());
        run(
            &p,
            &claude(),
            "SessionEnd",
            r#"{"session_id":"s1","reason":"other"}"#.as_bytes(),
        );
        let text = std::fs::read_to_string(queue_path(&p)).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["event"], "Stop");
        assert_eq!(lines[0]["agent"], "claude");
        assert_eq!(lines[0]["session_id"], "s1");
        assert_eq!(lines[0]["last_message_tail"], "Done. Shall I go on?");
        assert_eq!(lines[0]["background_tasks"], 2);
        assert!(!text.contains("SECRET PROMPT"), "prompts are not stored");
        assert_eq!(lines[1]["reason"], "other");
    }

    #[test]
    fn bad_stdin_logs_and_never_fails() {
        let (_t, p) = paths();
        run(&p, &claude(), "Stop", "not json".as_bytes());
        run(&p, &claude(), "Stop", "".as_bytes());
        assert!(!queue_path(&p).exists());
        let log = std::fs::read_to_string(p.home.join("logs/hook-errors.log")).unwrap();
        assert_eq!(log.lines().count(), 2);
    }

    #[test]
    fn huge_stdin_is_capped() {
        let (_t, p) = paths();
        let big = format!(
            r#"{{"session_id":"s","message":"{}"}}"#,
            "x".repeat(2 << 20)
        );
        run(&p, &claude(), "Notification", big.as_bytes());
        // cut off mid-string: invalid JSON, logged, no queue line
        assert!(!queue_path(&p).exists());
    }

    #[test]
    fn unwritable_home_is_swallowed() {
        let p = Paths {
            home: "/nonexistent/h".into(),
        };
        run(&p, &claude(), "Stop", r#"{"session_id":"s"}"#.as_bytes());
    }
}
