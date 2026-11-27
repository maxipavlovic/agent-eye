//! What Claude Code's prompt and recap text carries that is not the user's: built-in commands
//! ("/model opus"), the `<pasted_content>` tags around pasted text, and the tail it appends to
//! every recap.

use super::transcript::{clean_prompt, result_text};
use crate::util::head_chars;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static RE_PASTED_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<pasted_content\b[^>]*>(.*?)</pasted_content\b[^>]*>").expect("static regex")
});
static RE_PASTED_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?pasted_content\b[^>]*>").expect("static regex"));
static RE_SLASH_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/[A-Za-z][\w:.-]*$").expect("static regex"));
/// Built-in commands that take an argument and are still not a prompt ("/model opus").
const SLASH_BUILTINS: &[&str] = &[
    "/model",
    "/clear",
    "/config",
    "/compact",
    "/resume",
    "/effort",
    "/login",
    "/logout",
    "/mcp",
    "/permissions",
    "/status",
    "/help",
    "/exit",
    "/theme",
    "/fast",
    "/rewind",
    "/context",
];

/// What Claude Code writes in front of the user's own words when they refuse a tool call with a
/// comment: the comment is a prompt, typed inside the tool result.
const FEEDBACK_MARK: &str = "the user said:";

/// Claude Code appends this to every recap.
const RECAP_BOILERPLATE: &str = "(disable recaps in /config)";

/// A recap without Claude Code's "(disable recaps in /config)" tail.
pub fn clean_recap(t: &str) -> String {
    let t = t.trim_end();
    t.strip_suffix(RECAP_BOILERPLATE)
        .unwrap_or(t)
        .trim_end()
        .to_string()
}

/// True for "/model", "/clear", "/model opus": a command, not something the user asked.
fn is_slash_command(p: &str) -> bool {
    let p = p.trim();
    let mut it = p.splitn(2, char::is_whitespace);
    let name = it.next().unwrap_or("");
    RE_SLASH_ONLY.is_match(name)
        && (it.next().is_none_or(|a| a.trim().is_empty())
            || SLASH_BUILTINS.contains(&name.to_lowercase().as_str()))
}

/// A prompt as the detail panel shows it: paste tags removed, a prompt that is only pasted text
/// as "[pasted] " + its start, None for slash commands and empty leftovers.
pub fn display_prompt(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if is_slash_command(raw) {
        return None;
    }
    let outside = RE_PASTED_BLOCK.replace_all(raw, "");
    let outside = RE_PASTED_TAG.replace_all(&outside, "");
    if outside.trim().is_empty() {
        let inner: String = RE_PASTED_BLOCK
            .captures_iter(raw)
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        let inner = inner.split_whitespace().collect::<Vec<_>>().join(" ");
        if inner.is_empty() {
            return None;
        }
        let cut = head_chars(&inner, 100);
        let dots = if inner.chars().count() > 100 {
            "…"
        } else {
            ""
        };
        return Some(format!("[pasted] {cut}{dots}"));
    }
    let t = RE_PASTED_TAG.replace_all(raw, "");
    let t = t.trim();
    (!t.is_empty() && !is_slash_command(t)).then(|| t.to_string())
}

/// What the user typed in one transcript line, as the prompt the rest of eye counts: None for a
/// line that is not one (tool results, injected context, bare slash commands, background-task
/// notices). The turn events and the conversation reader both read prompts through this, so a
/// prompt the clock counts is one the correction pass can find. Besides a `user` line's text it
/// takes:
/// - the comment on a refused tool call, which sits in the tool result after "the user said:";
/// - a prompt typed while the agent worked, which only an `attachment` line of type
///   `queued_command` and human origin records.
pub(super) fn typed_prompt(v: &Value) -> Option<String> {
    let flag = |k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
    if flag("isMeta") || flag("isCompactSummary") {
        return None;
    }
    if v.get("type").and_then(Value::as_str) == Some("attachment") {
        let a = v.get("attachment")?;
        let typed = a.get("type").and_then(Value::as_str) == Some("queued_command")
            && a.pointer("/origin/kind").and_then(Value::as_str) == Some("human")
            && a.get("commandMode").and_then(Value::as_str) == Some("prompt")
            && a.get("isMeta").and_then(Value::as_bool) != Some(true);
        let text = a.get("prompt").and_then(Value::as_str).filter(|_| typed)?;
        return display_prompt(&clean_prompt(text)?);
    }
    let mut texts: Vec<String> = vec![];
    match v.get("message")?.get("content")? {
        Value::String(t) => texts.push(t.clone()),
        Value::Array(blocks) => {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => texts.extend(
                        b.get("text")
                            .and_then(Value::as_str)
                            .filter(|t| !t.starts_with("[Request interrupted"))
                            .map(String::from),
                    ),
                    Some("tool_result")
                        if b.get("is_error").and_then(Value::as_bool) == Some(true) =>
                    {
                        let result = result_text(b.get("content"));
                        if let Some((_, said)) = result.split_once(FEEDBACK_MARK) {
                            texts.push(said.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => return None,
    }
    if texts.is_empty() {
        return None;
    }
    display_prompt(&clean_prompt(&texts.join("\n"))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recap_boilerplate() {
        assert_eq!(
            clean_recap("Goal: ship it. Next: test.\n(disable recaps in /config)\n"),
            "Goal: ship it. Next: test."
        );
        assert_eq!(
            clean_recap("Goal: x (disable recaps in /config)"),
            "Goal: x"
        );
        assert_eq!(clean_recap("plain recap"), "plain recap");
    }
    #[test]
    fn slash_commands() {
        for c in ["/model", "/clear", " /model opus ", "/review-pr"] {
            assert!(is_slash_command(c), "{c}");
        }
        for c in [
            "/review-pr 12",
            "fix /model",
            "/etc/hosts is wrong",
            "hello",
        ] {
            assert!(!is_slash_command(c), "{c}");
        }
        assert_eq!(display_prompt("/clear"), None);
    }
    #[test]
    fn prompts_for_display() {
        assert_eq!(display_prompt("</pasted_content id=\"f1f0\">"), None);
        assert_eq!(
            display_prompt(
                "<pasted_content id=\"a1\">line one\n  line two</pasted_content id=\"a1\">"
            )
            .as_deref(),
            Some("[pasted] line one line two")
        );
        let long = format!(
            "<pasted_content id=\"a\">{}</pasted_content id=\"a\">",
            "x".repeat(300)
        );
        let d = display_prompt(&long).unwrap();
        assert_eq!(d.chars().count(), "[pasted] ".len() + 100 + 1);
        assert!(d.ends_with('…'));
        assert_eq!(
            display_prompt("why is <pasted_content id=\"b\">this</pasted_content id=\"b\"> slow?")
                .as_deref(),
            Some("why is this slow?")
        );
        assert_eq!(
            display_prompt("fix the bug").as_deref(),
            Some("fix the bug")
        );
    }

    #[test]
    fn what_the_user_typed_in_a_line_is_read_the_same_way_for_every_shape() {
        let user = |content: Value| json!({"type": "user", "message": {"content": content}});
        let said = "The user doesn't want to proceed with this tool use. To tell you how to proceed, the user said:\nuse rg instead";
        for (line, want) in [
            (user(json!("fix it")), Some("fix it")),
            (
                user(json!(
                    "<pasted_content id=\"a\">x</pasted_content id=\"a\"> why?"
                )),
                Some("x why?"),
            ),
            (
                user(json!(
                    "<command-name>/review-pr</command-name><command-args>12</command-args>"
                )),
                Some("/review-pr 12"),
            ),
            (
                user(json!([{"type": "tool_result", "is_error": true, "content": said}])),
                Some("use rg instead"),
            ),
            (
                user(json!([{"type": "tool_result", "is_error": true, "content": "Exit code 1"}])),
                None,
            ),
            (
                user(json!([{"type": "tool_result", "is_error": false, "content": said}])),
                None,
            ),
            (user(json!("/clear")), None),
            (user(json!("[Request interrupted by user]")), None),
            (
                json!({"type": "user", "isMeta": true, "message": {"content": "hi"}}),
                None,
            ),
        ] {
            assert_eq!(typed_prompt(&line).as_deref(), want, "{line}");
        }
        let queued = |origin: &str, mode: &str| {
            json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": "next, the tests",
                "commandMode": mode, "origin": {"kind": origin}}})
        };
        assert_eq!(
            typed_prompt(&queued("human", "prompt")).as_deref(),
            Some("next, the tests")
        );
        assert_eq!(typed_prompt(&queued("peer", "prompt")), None);
        assert_eq!(typed_prompt(&queued("human", "task-notification")), None);
    }
}
