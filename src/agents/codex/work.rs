//! The time of every message of a Codex rollout, for the work timeline.

use super::rollout::{read_line, Event};
use crate::agents::append_only::for_each_line;
use crate::util::iso_to_ms;
use crate::worked::WorkEvent;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// What a line is to the timeline: a prompt, or any other message of the thread (a reply, a tool
/// call or its result). The bookkeeping lines (meta, turn context, token totals) are not.
pub fn work_event(event: &Event, ts: i64) -> Option<WorkEvent> {
    let prompt = match event {
        Event::Prompt(_) => true,
        Event::Reply(_) | Event::Call { .. } | Event::Output { .. } | Event::Done { .. } => false,
        _ => return None,
    };
    Some(WorkEvent { ts, prompt })
}

/// The events of the whole rollout at `path`.
pub fn read_events(path: &Path) -> Result<Vec<WorkEvent>> {
    let mut events = vec![];
    for_each_line(path, 0, |line| {
        let Some((v, event)) = read_line(line) else {
            return;
        };
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms);
        if let Some(e) = ts.and_then(|t| work_event(&event, t)) {
            events.push(e);
        }
    })?;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(ts: &str, ty: &str, payload: Value) -> String {
        json!({"timestamp": ts, "type": ty, "payload": payload}).to_string()
    }

    #[test]
    fn prompts_replies_and_tool_calls_are_events_and_bookkeeping_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let text = [
            line(
                "2026-11-01T10:00:00.000Z",
                "session_meta",
                json!({"id":"abc","cwd":"/tmp/demo"}),
            ),
            line(
                "2026-11-01T10:00:01.000Z",
                "event_msg",
                json!({"type":"user_message","message":"fix it"}),
            ),
            line(
                "2026-11-01T10:00:02.000Z",
                "turn_context",
                json!({"cwd":"/tmp/demo"}),
            ),
            line(
                "2026-11-01T10:01:00.000Z",
                "event_msg",
                json!({"type":"agent_message","message":"done"}),
            ),
        ]
        .join("\n")
            + "\n";
        std::fs::write(&path, text).unwrap();
        let events = read_events(&path).unwrap();
        assert_eq!(
            events.iter().map(|e| e.prompt).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!(events[1].ts - events[0].ts, 59_000);
    }
}
