//! Claude Code hook payloads: what is kept of one, and what it says about a session's wait.

use super::SHORT;
use crate::agents::questions::{asks_user, last_question};
use crate::agents::Wait;
use crate::summarize::mask_secrets;
use crate::util::{head_chars, tail_chars};
use serde_json::{json, Map, Value};

/// Keys copied verbatim when they arrive as short strings.
const KEYS: &[&str] = &[
    "session_id",
    "transcript_path",
    "cwd",
    "notification_type",
    "message",
    "source",
    "reason",
    "agent_id",
    "agent_type",
    "agent_transcript_path",
    "tool_name",
    "permission_mode",
    "prompt_id",
];

/// Turn a hook payload into a queue line. `event` is the name given on the command line.
pub fn queue_line(event: &str, payload: &Value, ts: i64) -> Value {
    let mut m = Map::new();
    m.insert("ts".into(), json!(ts));
    let name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or(event);
    m.insert(
        "event".into(),
        json!(if event.is_empty() { name } else { event }),
    );
    for k in KEYS {
        if let Some(v) = payload.get(*k).and_then(Value::as_str) {
            m.insert((*k).into(), json!(head_chars(v, 600)));
        }
    }
    // Stop: the end of the last reply tells whether the agent is asking the user something.
    if let Some(t) = payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
    {
        m.insert(
            "last_message_tail".into(),
            json!(tail_chars(t.trim_end(), 300)),
        );
    }
    // PermissionRequest: what the agent wants, short enough for a card ("You: answer: …").
    if let (Some(tool), Some(input)) = (
        payload.get("tool_name").and_then(Value::as_str),
        payload.get("tool_input"),
    ) {
        if let Some(d) = tool_detail(tool, input) {
            m.insert("tool_detail".into(), json!(d));
        }
    }
    if let Some(n) = payload
        .get("background_tasks")
        .and_then(Value::as_array)
        .map(Vec::len)
    {
        m.insert("background_tasks".into(), json!(n));
    }
    Value::Object(m)
}

/// The part of a tool call a person decides on: the question asked, the command, the file or the
/// URL. One line, at most 240 characters, secrets masked.
fn tool_detail(tool: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let t = match tool {
        "AskUserQuestion" => input
            .get("questions")?
            .as_array()?
            .iter()
            .filter_map(|q| q.get("question").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" / "),
        "ExitPlanMode" => return None,
        _ => s("command")
            .or_else(|| s("file_path"))
            .or_else(|| s("notebook_path"))
            .or_else(|| s("url"))
            .or_else(|| s("query"))
            .or_else(|| s("description"))?
            .to_string(),
    };
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then(|| head_chars(&mask_secrets(&t), 240))
}

/// None: the event does not change the wait. "Claude is waiting for your input" (idle_prompt)
/// comes a minute after every finished reply; the Stop before it already said whether the reply
/// asks something, so it changes nothing.
pub fn wait_effect(kind: &str, ev: &Value, ts: i64) -> Option<Wait> {
    let s = |k: &str| ev.get(k).and_then(Value::as_str);
    let ask = |note: String| Wait {
        since: Some(ts),
        note: Some(note),
        prompt: true,
        only_if_prompt: false,
    };
    match kind {
        "SessionStart" | "UserPromptSubmit" | "SessionEnd" => Some(Wait {
            since: None,
            note: None,
            prompt: false,
            only_if_prompt: false,
        }),
        "PermissionRequest" => Some(ask(match (s("tool_name"), s("tool_detail")) {
            (Some("AskUserQuestion"), Some(q)) => format!("answer: {q}"),
            (Some("AskUserQuestion"), None) => format!("answer {SHORT}'s question in the session"),
            (Some("ExitPlanMode"), _) => "approve or change the plan".into(),
            (Some(t), Some(d)) => format!("allow {t}: {d}"),
            (Some(t), None) => format!("allow {t}"),
            (None, _) => "answer the permission prompt".into(),
        })),
        "Notification" => match s("notification_type") {
            Some("idle_prompt") => None,
            Some("permission_prompt") => Some(ask("answer the permission prompt".into())),
            _ => Some(ask(s("message")
                .map(String::from)
                .unwrap_or_else(|| format!("answer {SHORT} in the session")))),
        },
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
    fn permission_requests_keep_what_is_asked() {
        let line = |tool: &str, input: Value| {
            queue_line(
                "PermissionRequest",
                &json!({"session_id": "s", "tool_name": tool, "tool_input": input}),
                1,
            )
        };
        let q = line(
            "AskUserQuestion",
            json!({"questions": [{"question": "Which repo first?", "options": []}, {"question": "Keep the draft?"}]}),
        );
        assert_eq!(q["tool_detail"], "Which repo first? / Keep the draft?");
        let b = line(
            "Bash",
            json!({"command": "curl -H 'Authorization: Bearer abcdefgh12345678'\n  https://x"}),
        );
        let b = b["tool_detail"].as_str().unwrap();
        assert!(
            b.starts_with("curl -H") && !b.contains("abcdefgh12345678") && !b.contains('\n'),
            "{b}"
        );
        assert_eq!(
            line("Edit", json!({"file_path": "/x/a.rs", "old_string": "zz"}))["tool_detail"],
            "/x/a.rs"
        );
        assert!(line("ExitPlanMode", json!({"plan": "long"}))
            .get("tool_detail")
            .is_none());
        // the full input itself is never queued
        assert!(line("Edit", json!({"file_path": "/x", "old_string": "zz"}))
            .get("tool_input")
            .is_none());
    }

    #[test]
    fn a_reply_that_asks_waits_and_idle_prompts_change_nothing() {
        let stop = json!({"last_message_tail": "Done with A. Which repo first?"});
        let w = wait_effect("Stop", &stop, 5).unwrap();
        assert_eq!(
            (w.since, w.note.as_deref(), w.prompt),
            (Some(5), Some("answer: Which repo first?"), false)
        );
        let done = wait_effect("Stop", &json!({"last_message_tail": "All done."}), 6).unwrap();
        assert_eq!((done.since, done.note), (None, None));
        let idle = json!({"notification_type": "idle_prompt", "message": "waiting for your input"});
        assert!(wait_effect("Notification", &idle, 7).is_none());
        let ask = wait_effect(
            "PermissionRequest",
            &json!({"tool_name": "AskUserQuestion"}),
            8,
        )
        .unwrap();
        assert_eq!(
            ask.note.as_deref(),
            Some("answer Claude's question in the session")
        );
        assert!(ask.prompt);
    }
}
