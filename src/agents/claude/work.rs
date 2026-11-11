//! The time of every message of a Claude Code transcript, for the work timeline.

use super::super::append_only::for_each_line;
use super::transcript::{clean_prompt, line_type};
use crate::util::iso_to_ms;
use crate::worked::WorkEvent;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Longest line that is parsed when its type cannot be told from its start.
const NO_TYPE_PARSE_MAX: usize = 64 << 20;

/// What a transcript line is to the timeline: a prompt (a user line with text that is not
/// injected), a reply or a tool result; None for anything else (a sub-agent's line, a line the
/// harness added, a summary of compacted history). `ts` is the line's time.
pub(super) fn work_event(v: &Value, ty: &str, ts: i64) -> Option<WorkEvent> {
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let prompt = match ty {
        "assistant" => false,
        "user" => {
            let flag = |k| v.get(k).and_then(Value::as_bool) == Some(true);
            if flag("isMeta") || flag("isCompactSummary") {
                return None;
            }
            is_prompt(v.get("message")?.get("content")?)
        }
        _ => return None,
    };
    Some(WorkEvent { ts, prompt })
}

/// Does the content of a user line hold something the user typed?
fn is_prompt(content: &Value) -> bool {
    let text = match content {
        Value::String(t) => t.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return false,
    };
    !text.is_empty() && clean_prompt(&text).is_some()
}

/// The events of the whole transcript at `path`; only the lines that can be messages are parsed.
pub(super) fn read_events(path: &Path) -> Result<Vec<WorkEvent>> {
    let mut events = vec![];
    for_each_line(path, 0, |line| {
        if !matches!(line_type(line), None | Some("user" | "assistant")) {
            return;
        }
        if line_type(line).is_none() && line.len() > NO_TYPE_PARSE_MAX {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms);
        if let Some(e) = ts.and_then(|t| work_event(&v, ty, t)) {
            events.push(e);
        }
    })?;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T: i64 = 1_000;

    fn event(v: Value) -> Option<WorkEvent> {
        let ty = v["type"].as_str().unwrap().to_string();
        work_event(&v, &ty, T)
    }

    #[test]
    fn what_the_user_typed_is_a_prompt() {
        let typed = json!({"type":"user","message":{"content":"fix the build"}});
        assert_eq!(
            event(typed),
            Some(WorkEvent {
                ts: T,
                prompt: true
            })
        );
        let blocks =
            json!({"type":"user","message":{"content":[{"type":"text","text":"and this"}]}});
        assert_eq!(
            event(blocks),
            Some(WorkEvent {
                ts: T,
                prompt: true
            })
        );
    }

    #[test]
    fn a_tool_result_a_notification_and_a_reply_are_not_prompts() {
        let result = json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"ok"}]}});
        assert_eq!(
            event(result),
            Some(WorkEvent {
                ts: T,
                prompt: false
            })
        );
        let notice = json!({"type":"user","message":{"content":"<task-notification>done</task-notification>"}});
        assert_eq!(
            event(notice),
            Some(WorkEvent {
                ts: T,
                prompt: false
            })
        );
        let reply = json!({"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}});
        assert_eq!(
            event(reply),
            Some(WorkEvent {
                ts: T,
                prompt: false
            })
        );
    }

    #[test]
    fn lines_the_harness_adds_and_sub_agent_lines_are_not_messages() {
        let meta = json!({"type":"user","isMeta":true,"message":{"content":"<command-name>/x</command-name>"}});
        assert_eq!(event(meta), None);
        let compact =
            json!({"type":"user","isCompactSummary":true,"message":{"content":"summary"}});
        assert_eq!(event(compact), None);
        let side = json!({"type":"assistant","isSidechain":true,"message":{"content":[]}});
        assert_eq!(event(side), None);
        assert_eq!(event(json!({"type":"system","content":"x"})), None);
    }

    #[test]
    fn a_transcript_file_gives_the_same_events_as_its_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let text = [
            r#"{"type":"user","timestamp":"2026-11-01T10:00:00.000Z","message":{"content":"go"}}"#,
            r#"{"type":"attachment","timestamp":"2026-11-01T10:00:01.000Z"}"#,
            r#"{"type":"assistant","timestamp":"2026-11-01T10:01:00.000Z","message":{"content":[{"type":"text","text":"ok"}]}}"#,
            r#"{"type":"user","timestamp":"2026-11-01T10:02:00.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"x"}]}}"#,
            r#"{"type":"user","message":{"content":"no time"}}"#,
        ]
        .join("\n")
            + "\n";
        std::fs::write(&path, &text).unwrap();
        let read = read_events(&path).unwrap();
        let tokens = std::collections::HashSet::new();
        let parsed = super::super::transcript::parse_reader(
            std::io::Cursor::new(text.into_bytes()),
            0,
            &crate::agents::ParseOpts {
                job_tokens: &tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            },
        )
        .unwrap();
        assert_eq!(read, parsed.events);
        assert_eq!(read.iter().filter(|e| e.prompt).count(), 1);
        assert_eq!(read.len(), 3);
    }
}
