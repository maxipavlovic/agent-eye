//! What the plugin eye installs in OpenCode (`plugin.js`) hands to `agent-eye hook <Event> --agent
//! opencode`, and what each event does to a session's wait. The payloads have the shape of
//! Claude Code's hook payloads (`session_id`, `cwd`, `tool_name`, ...), with `resources` (what a
//! permission is asked for) and `reply` (how it was answered) added.

use super::SHORT;
use crate::agents::questions::{asks_user, last_question};
use crate::agents::Wait;
use crate::summarize::mask_secrets;
use crate::util::{head_chars, tail_chars};
use serde_json::{json, Map, Value};

/// The events the plugin sends. Anything else is not queued.
pub const EVENTS: [&str; 5] = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PermissionReplied",
    "Stop",
];

/// Turn a payload into a queue line, None for an event eye does not use.
pub fn queue_line(event: &str, payload: &Value, ts: i64) -> Option<Value> {
    if !EVENTS.contains(&event) {
        return None;
    }
    let s = |k: &str| payload.get(k).and_then(Value::as_str);
    let mut m = Map::new();
    m.insert("ts".into(), json!(ts));
    m.insert("event".into(), json!(event));
    for k in ["session_id", "cwd", "tool_name", "reason", "reply"] {
        if let Some(v) = s(k).filter(|v| !v.is_empty()) {
            m.insert(k.into(), json!(head_chars(v, 600)));
        }
    }
    if let Some(t) = s("last_assistant_message").filter(|t| !t.trim().is_empty()) {
        m.insert(
            "last_message_tail".into(),
            json!(tail_chars(t.trim_end(), 300)),
        );
    }
    if let Some(d) = payload
        .get("resources")
        .and_then(Value::as_array)
        .and_then(|r| detail(s("tool_name").unwrap_or(""), r))
    {
        m.insert("tool_detail".into(), json!(d));
    }
    Some(Value::Object(m))
}

/// What a permission is asked for, as one line of at most 240 characters with secrets masked: the
/// command, or `edit <files>`.
fn detail(action: &str, resources: &[Value]) -> Option<String> {
    let parts: Vec<&str> = resources.iter().filter_map(Value::as_str).collect();
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(", ");
    let t = if matches!(action, "shell" | "bash") {
        joined
    } else if action == "edit" {
        format!("edit {joined}")
    } else {
        format!("{action} {joined}")
    };
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then(|| head_chars(&mask_secrets(&t), 240))
}

fn clear() -> Wait {
    Wait {
        since: None,
        note: None,
        prompt: false,
        only_if_prompt: false,
    }
}

/// None: the event does not change the wait. A permission request stops the turn until it is
/// answered, and the plugin says when it was (`PermissionReplied`); a finished turn waits on you
/// only when its last message is a question.
pub fn wait_effect(kind: &str, ev: &Value, ts: i64) -> Option<Wait> {
    let s = |k: &str| ev.get(k).and_then(Value::as_str);
    match kind {
        "SessionStart" | "UserPromptSubmit" | "PermissionReplied" => Some(clear()),
        "PermissionRequest" => Some(Wait {
            since: Some(ts),
            note: Some(match (s("tool_name"), s("tool_detail")) {
                (Some(t), Some(d)) => format!("allow {t}: {d}"),
                (Some(t), None) => format!("allow {t}"),
                (None, _) => format!("answer {SHORT}'s permission prompt"),
            }),
            prompt: true,
            only_if_prompt: false,
        }),
        "Stop" => {
            let q = s("last_message_tail").filter(|t| asks_user(t));
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
    fn a_permission_request_keeps_what_is_asked_and_masks_secrets() {
        let q = queue_line(
            "PermissionRequest",
            &json!({"session_id": "ses_1", "tool_name": "shell",
                    "resources": ["curl -H 'Authorization: Bearer abcdefgh12345678'\n  https://x"],
                    "prompt": "SECRET PROMPT"}),
            9,
        )
        .unwrap();
        let d = q["tool_detail"].as_str().unwrap();
        assert!(d.starts_with("curl -H") && !d.contains("abcdefgh12345678") && !d.contains('\n'));
        assert_eq!(
            (q["event"].as_str(), q["session_id"].as_str()),
            (Some("PermissionRequest"), Some("ses_1"))
        );
        assert!(q.get("prompt").is_none() && q.get("resources").is_none());
        let e = queue_line(
            "PermissionRequest",
            &json!({"tool_name": "edit", "resources": ["a.py", "b.py"]}),
            1,
        )
        .unwrap();
        assert_eq!(e["tool_detail"], "edit a.py, b.py");
    }

    #[test]
    fn events_eye_does_not_use_are_not_queued() {
        assert!(queue_line("session.text.delta", &json!({"session_id": "s"}), 1).is_none());
        assert!(queue_line("PostToolUse", &json!({"session_id": "s"}), 1).is_none());
    }

    #[test]
    fn the_wait_follows_the_turn() {
        let ask = wait_effect(
            "PermissionRequest",
            &json!({"tool_name": "shell", "tool_detail": "rm -rf build"}),
            5,
        )
        .unwrap();
        assert_eq!(ask.note.as_deref(), Some("allow shell: rm -rf build"));
        assert!(ask.prompt && ask.since == Some(5));
        let bare = wait_effect("PermissionRequest", &json!({}), 5).unwrap();
        assert_eq!(
            bare.note.as_deref(),
            Some("answer OpenCode's permission prompt")
        );
        for clears in ["SessionStart", "UserPromptSubmit", "PermissionReplied"] {
            assert_eq!(
                wait_effect(clears, &json!({}), 6).unwrap(),
                clear(),
                "{clears}"
            );
        }
        let stop = wait_effect(
            "Stop",
            &json!({"last_message_tail": "Done with A. Which repo first?"}),
            7,
        )
        .unwrap();
        assert_eq!(
            (stop.since, stop.note.as_deref()),
            (Some(7), Some("answer: Which repo first?"))
        );
        let done = wait_effect("Stop", &json!({"last_message_tail": "All done."}), 8).unwrap();
        assert_eq!((done.since, done.note, done.prompt), (None, None, false));
        assert!(wait_effect("PostToolUse", &json!({}), 1).is_none());
    }
}
