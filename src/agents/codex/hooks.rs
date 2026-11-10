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
    "source",
    "reason",
    "tool_name",
    "permission_mode",
    "turn_id",
];

/// Turn a hook payload into a queue line. `event` is the name given on the command line.
pub fn queue_line(event: &str, payload: &Value, ts: i64) -> Value {
    let mut m = Map::new();
    m.insert("ts".into(), json!(ts));
    let name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or(event);
    let kind = if event.is_empty() { name } else { event };
    m.insert("event".into(), json!(kind));
    if kind == "PostToolUse" {
        // It comes after every tool call and carries the tool's output: only whose call it was is kept.
        if let Some(v) = payload.get("session_id").and_then(Value::as_str) {
            m.insert("session_id".into(), json!(head_chars(v, 600)));
        }
        return Value::Object(m);
    }
    for k in KEYS {
        if let Some(v) = payload.get(*k).and_then(Value::as_str) {
            m.insert((*k).into(), json!(head_chars(v, 600)));
        }
    }
    if let Some(t) = payload
        .get("last_assistant_message")
        .and_then(Value::as_str)
    {
        m.insert(
            "last_message_tail".into(),
            json!(tail_chars(t.trim_end(), 300)),
        );
    }
    if let (Some(tool), Some(input)) = (
        payload.get("tool_name").and_then(Value::as_str),
        payload.get("tool_input"),
    ) {
        if let Some(d) = tool_detail(tool, input) {
            m.insert("tool_detail".into(), json!(d));
        }
    }
    Value::Object(m)
}

/// The part of a tool call a person decides on: the command, or the files a patch changes. One
/// line, at most 240 characters, secrets masked.
fn tool_detail(tool: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let t = if tool == "apply_patch" {
        let files = super::rollout::patch_files(s("command").or_else(|| s("input"))?);
        format!("edit {}", files.join(", "))
    } else {
        s("command").or_else(|| s("description"))?.to_string()
    };
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    (!t.is_empty()).then(|| head_chars(&mask_secrets(&t), 240))
}

/// None: the event does not change the wait. A permission request stops Codex until you answer
/// it, and the next tool that finishes shows it was answered; a finished turn waits on you only
/// when its last message is a question.
pub fn wait_effect(kind: &str, ev: &Value, ts: i64) -> Option<Wait> {
    let s = |k: &str| ev.get(k).and_then(Value::as_str);
    match kind {
        "SessionStart" | "UserPromptSubmit" | "SessionEnd" => Some(Wait {
            since: None,
            note: None,
            prompt: false,
            only_if_prompt: false,
        }),
        "PostToolUse" => Some(Wait {
            since: None,
            note: None,
            prompt: false,
            only_if_prompt: true,
        }),
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
    fn a_permission_request_keeps_what_is_asked_and_nothing_else() {
        let line = |tool: &str, input: Value| {
            queue_line(
                "PermissionRequest",
                &json!({"session_id": "s", "turn_id": "t", "tool_name": tool, "tool_input": input,
                        "prompt": "SECRET PROMPT", "cwd": "/w", "transcript_path": null, "model": "gpt-6"}),
                1,
            )
        };
        let b = line(
            "Bash",
            json!({"command": "curl -H 'Authorization: Bearer abcdefgh12345678'\n  https://x", "description": "fetch"}),
        );
        let d = b["tool_detail"].as_str().unwrap();
        assert!(
            d.starts_with("curl -H") && !d.contains("abcdefgh12345678") && !d.contains('\n'),
            "{d}"
        );
        assert_eq!(b["session_id"], "s");
        assert_eq!(b["cwd"], "/w");
        assert!(
            b.get("transcript_path").is_none(),
            "a null path is left out"
        );
        assert!(b.get("prompt").is_none() && b.get("tool_input").is_none());
        let p = line(
            "apply_patch",
            json!({"command": "*** Begin Patch\n*** Update File: a.py\n*** Add File: b/c.py\n*** End Patch"}),
        );
        assert_eq!(p["tool_detail"], "edit a.py, b/c.py");
        assert!(line("mcp__docs__search", json!({"query": "x"}))
            .get("tool_detail")
            .is_none());
    }

    #[test]
    fn a_stop_keeps_the_tail_of_the_last_message() {
        let q = queue_line(
            "Stop",
            &json!({"session_id": "s", "last_assistant_message": "Done. Which repo first?"}),
            2,
        );
        assert_eq!(q["last_message_tail"], "Done. Which repo first?");
        assert_eq!(q["event"], "Stop");
    }

    #[test]
    fn a_reply_that_asks_waits_and_a_permission_request_blocks() {
        let stop = json!({"last_message_tail": "Done with A. Which repo first?"});
        let w = wait_effect("Stop", &stop, 5).unwrap();
        assert_eq!(
            (w.since, w.note.as_deref(), w.prompt),
            (Some(5), Some("answer: Which repo first?"), false)
        );
        let done = wait_effect("Stop", &json!({"last_message_tail": "All done."}), 6).unwrap();
        assert_eq!((done.since, done.note), (None, None));
        let ask = wait_effect(
            "PermissionRequest",
            &json!({"tool_name": "Bash", "tool_detail": "rm -rf build"}),
            8,
        )
        .unwrap();
        assert_eq!(ask.note.as_deref(), Some("allow Bash: rm -rf build"));
        assert!(ask.prompt && ask.since == Some(8));
        let bare = wait_effect("PermissionRequest", &json!({}), 9).unwrap();
        assert_eq!(
            bare.note.as_deref(),
            Some("answer Codex's permission prompt")
        );
        for clears in ["SessionStart", "UserPromptSubmit", "SessionEnd"] {
            let w = wait_effect(clears, &json!({}), 3).unwrap();
            assert_eq!((w.since, w.note, w.prompt), (None, None, false), "{clears}");
        }
        assert!(wait_effect("PreToolUse", &json!({}), 3).is_none());
    }

    #[test]
    fn a_tool_that_ran_ends_a_prompt_and_nothing_else() {
        let ran = wait_effect("PostToolUse", &json!({"tool_name": "Bash"}), 4).unwrap();
        assert_eq!(
            (ran.since, ran.note, ran.prompt, ran.only_if_prompt),
            (None, None, false, true)
        );
        for other in ["SessionStart", "UserPromptSubmit", "SessionEnd", "Stop"] {
            let w = wait_effect(other, &json!({}), 4).unwrap();
            assert!(!w.only_if_prompt, "{other}");
        }
        assert!(
            !wait_effect("PermissionRequest", &json!({}), 4)
                .unwrap()
                .only_if_prompt
        );
    }

    #[test]
    fn a_tool_that_ran_is_queued_without_its_input_or_output() {
        let line = queue_line(
            "PostToolUse",
            &json!({"session_id": "s", "turn_id": "t", "hook_event_name": "PostToolUse",
                    "tool_name": "Bash", "cwd": "/w", "transcript_path": "/w/r.jsonl",
                    "tool_input": {"command": "cat secrets.env"},
                    "tool_response": {"output": "TOKEN=abcdefgh12345678", "exit_code": 0}}),
            7,
        );
        assert_eq!(
            line,
            json!({"ts": 7, "event": "PostToolUse", "session_id": "s"})
        );
    }
}
