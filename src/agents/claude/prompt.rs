//! What Claude Code's prompt and recap text carries that is not the user's: built-in commands
//! ("/model opus"), the `<pasted_content>` tags around pasted text, and the tail it appends to
//! every recap.

use crate::util::head_chars;
use regex::Regex;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
