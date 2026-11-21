//! What agy's hooks tell eye. agy runs a hook command with a JSON object on stdin (camelCase keys:
//! `conversationId`, `cwd`, `transcriptPath`, `hookEventName`, and for `Stop` `terminationReason`).
//! eye listens to two events, `SessionStart` (a new conversation: nothing is asked of you) and
//! `Stop` (a turn ended; the reply it ended with may ask you something). The queue line keeps no
//! prompt and no tool output.

use crate::agents::questions::{asks_user, last_question};
use crate::agents::Wait;
use crate::util::{head_chars, tail_chars};
use serde_json::{json, Map, Value};

/// The events eye installs a hook for.
pub const EVENTS: [&str; 2] = ["SessionStart", "Stop"];

/// Turn a hook payload into a queue line. `event` is the name given on the command line.
pub fn queue_line(event: &str, payload: &Value, ts: i64) -> Value {
    let s = |k: &str| payload.get(k).and_then(Value::as_str);
    let mut m = Map::new();
    m.insert("ts".into(), json!(ts));
    let kind = if event.is_empty() {
        s("hookEventName").unwrap_or("")
    } else {
        event
    };
    m.insert("event".into(), json!(kind));
    if let Some(v) = s("conversationId") {
        m.insert("session_id".into(), json!(head_chars(v, 600)));
    }
    if let Some(v) = s("cwd") {
        m.insert("cwd".into(), json!(head_chars(v, 600)));
    }
    if kind == "Stop" {
        if let Some(r) = s("terminationReason") {
            m.insert("reason".into(), json!(head_chars(r, 80)));
        }
        if let Some(t) =
            s("transcriptPath").and_then(|p| super::log::last_reply(std::path::Path::new(p)))
        {
            m.insert(
                "last_message_tail".into(),
                json!(tail_chars(t.trim_end(), 300)),
            );
        }
    }
    Value::Object(m)
}

/// None: the event does not change the wait. A new conversation clears it; a finished turn waits
/// on you only when its last message is a question (and you did not cancel it).
pub fn wait_effect(kind: &str, ev: &Value, ts: i64) -> Option<Wait> {
    let s = |k: &str| ev.get(k).and_then(Value::as_str);
    match kind {
        "SessionStart" => Some(Wait {
            since: None,
            note: None,
            prompt: false,
            only_if_prompt: false,
        }),
        "Stop" => {
            let q = s("last_message_tail")
                .filter(|t| asks_user(t) && s("reason") != Some("USER_CANCELED"));
            Some(Wait {
                since: q.map(|_| ts),
                note: q.map(|t| format!("answer: {}", last_question(t))),
                prompt: false,
                only_if_prompt: false,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stop_line_keeps_the_session_and_the_end_of_the_last_reply_only() {
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("transcript_full.jsonl");
        std::fs::write(
            &log,
            concat!(
                r#"{"step_index":0,"type":"USER_INPUT","content":"<USER_REQUEST>\nSECRET PROMPT\n</USER_REQUEST>"}"#,
                "\n",
                r#"{"step_index":1,"type":"PLANNER_RESPONSE","content":"Which branch should I use?"}"#,
                "\n",
            ),
        )
        .unwrap();
        let line = queue_line(
            "Stop",
            &json!({"conversationId": "c1", "cwd": "/w", "transcriptPath": log,
                    "terminationReason": "NO_TOOL_CALL", "lastUserInput": "SECRET PROMPT",
                    "modelName": "m"}),
            7,
        );
        assert_eq!(line["session_id"], "c1");
        assert_eq!(line["last_message_tail"], "Which branch should I use?");
        assert!(!line.to_string().contains("SECRET"));
        let w = wait_effect("Stop", &line, 7).unwrap();
        assert_eq!(w.since, Some(7));
        assert!(w.note.unwrap().starts_with("answer: "));
    }

    #[test]
    fn a_cancelled_or_plain_stop_waits_on_nobody() {
        let q =
            json!({"event": "Stop", "last_message_tail": "Which one?", "reason": "USER_CANCELED"});
        assert_eq!(wait_effect("Stop", &q, 1).unwrap().since, None);
        let plain = json!({"event": "Stop", "last_message_tail": "Done."});
        assert_eq!(wait_effect("Stop", &plain, 1).unwrap().since, None);
        assert_eq!(wait_effect("SessionStart", &plain, 1).unwrap().since, None);
        assert!(wait_effect("PostToolUse", &plain, 1).is_none());
    }

    #[test]
    fn the_event_name_comes_from_the_command_line_else_the_payload() {
        let p = json!({"hookEventName": "SessionStart", "conversationId": "c"});
        assert_eq!(queue_line("", &p, 1)["event"], "SessionStart");
        assert_eq!(queue_line("Stop", &p, 1)["event"], "Stop");
    }
}
