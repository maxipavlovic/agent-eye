//! Claude Code transcripts read as a conversation (real prompts and replies) and as the tail
//! for a summary excerpt (the last messages with one line per tool call).

use super::prompt::display_prompt;
use super::transcript::clean_prompt;
use crate::agents::TailItem;
use crate::conversation::Message;
use crate::util::{iso_to_ms, tail_chars};
use anyhow::Result;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::BufRead;

const MSG_CAP: usize = 1_500;
const TAIL_TEXTS: usize = 30;
const TAIL_ITEMS_MAX: usize = 90;
const LINE_MAX: usize = 64 << 20;

/// The user's prompts and Claude's replies (text blocks of one turn joined, with the tool-call
/// count). Sub-agent lines, tool results and injected lines are left out.
pub fn conversation(r: &mut dyn BufRead) -> Vec<Message> {
    let mut out: Vec<Message> = vec![];
    let mut turn = Turn::default();
    let mut buf: Vec<u8> = vec![];
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        let user = contains(line, b"\"type\":\"user\"");
        let assistant = !user && contains(line, b"\"type\":\"assistant\"");
        if !user && !assistant {
            continue;
        }
        // A tool result is the bulk of a transcript and never a prompt.
        if user
            && contains(line, b"\"type\":\"tool_result\"")
            && !contains(line, b"\"type\":\"text\"")
        {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms);
        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(text) = prompt_of(&v) {
                    turn.flush(&mut out);
                    out.push(Message::new("user", &text, ts, 0));
                }
            }
            Some("assistant") => turn.add(&v, ts),
            _ => {}
        }
    }
    turn.flush(&mut out);
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The text of a real prompt, None for tool results, injected lines and slash commands.
fn prompt_of(v: &Value) -> Option<String> {
    let flag = |k: &str| v.get(k).and_then(Value::as_bool) == Some(true);
    if flag("isMeta") || flag("isCompactSummary") {
        return None;
    }
    let texts: Vec<&str> = match v.get("message")?.get("content")? {
        Value::String(t) => vec![t.as_str()],
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect(),
        _ => return None,
    };
    if texts.is_empty() {
        return None;
    }
    display_prompt(&clean_prompt(&texts.join("\n"))?)
}

/// Claude's lines since the last prompt.
#[derive(Default)]
struct Turn {
    texts: Vec<String>,
    tools: u32,
    ts: Option<i64>,
    any: bool,
}

impl Turn {
    fn add(&mut self, v: &Value, ts: Option<i64>) {
        let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) else {
            return;
        };
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = b
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|t| !t.trim().is_empty())
                    {
                        self.texts.push(t.trim().to_string());
                        self.any = true;
                    }
                }
                Some("tool_use") => {
                    self.tools += 1;
                    self.any = true;
                }
                _ => {}
            }
        }
        self.ts = ts.or(self.ts);
    }

    fn flush(&mut self, out: &mut Vec<Message>) {
        let t = std::mem::take(self);
        if t.any {
            out.push(Message::new(
                "assistant",
                &t.texts.join("\n\n"),
                t.ts,
                t.tools,
            ));
        }
    }
}

fn one_line(s: &str, max: usize) -> String {
    let t: String = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect();
    t
}

fn tool_line(name: &str, input: &Value) -> Option<String> {
    let g = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    match name {
        "Bash" => Some(format!("Bash: {}", one_line(g("command"), 120))),
        "Edit" | "Write" | "NotebookEdit" => {
            let p = if g("file_path").is_empty() {
                g("notebook_path")
            } else {
                g("file_path")
            };
            Some(format!("{name}: {p}"))
        }
        "Artifact" => {
            let t = [g("title"), g("url"), g("file_path")]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("");
            Some(format!("Artifact: {} {}", g("action"), one_line(t, 100)))
        }
        "Agent" | "Task" => Some(format!("Agent: {}", one_line(g("description"), 100))),
        _ => None,
    }
}

fn result_error(b: &Value) -> Option<String> {
    if b.get("is_error").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let text = match b.get("content") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    Some(format!("error: {}", one_line(&text, 160)))
}

/// Stream the transcript and keep the last ~30 messages (with the tool calls between them).
pub fn tail(r: &mut dyn BufRead) -> Result<Vec<TailItem>> {
    let mut buf = Vec::new();
    let mut items: VecDeque<TailItem> = VecDeque::new();
    let mut texts = 0usize;
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        // lines that start with their type (attachments, api-request, ...) are never user/assistant
        if buf.starts_with(b"{\"type\":\"") || buf.len() > LINE_MAX {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        let role = match v.get("type").and_then(Value::as_str) {
            Some("user") => "user",
            Some("assistant") => "assistant",
            _ => continue,
        };
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let meta = v.get("isMeta").and_then(Value::as_bool) == Some(true)
            || v.get("isCompactSummary").and_then(Value::as_bool) == Some(true);
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let mut new: Vec<TailItem> = Vec::new();
        let mut text_parts: Vec<&str> = Vec::new();
        match content {
            Value::String(t) => text_parts.push(t),
            Value::Array(blocks) => {
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                text_parts.push(t);
                            }
                        }
                        Some("tool_use") => {
                            let name = b.get("name").and_then(Value::as_str).unwrap_or("");
                            let input = b.get("input").unwrap_or(&Value::Null);
                            if let Some(l) = tool_line(name, input) {
                                new.push(TailItem::Tool(l));
                            }
                        }
                        Some("tool_result") => {
                            if let Some(e) = result_error(b) {
                                new.push(TailItem::Tool(e));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        let joined = text_parts.join("\n");
        let text = if role == "user" {
            if meta {
                None
            } else {
                clean_prompt(&joined)
            }
        } else if joined.trim().is_empty() {
            None
        } else {
            Some(joined.trim().to_string())
        };
        if let Some(t) = text {
            new.insert(
                0,
                TailItem::Text {
                    role,
                    text: tail_chars(&t, MSG_CAP),
                },
            );
        }
        for it in new {
            if matches!(it, TailItem::Text { .. }) {
                texts += 1;
            }
            items.push_back(it);
        }
        while texts > TAIL_TEXTS || items.len() > TAIL_ITEMS_MAX {
            match items.pop_front() {
                Some(TailItem::Text { .. }) => texts -= 1,
                Some(_) => {}
                None => break,
            }
        }
    }
    Ok(items.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::TEXT_CAP;
    use std::io::Cursor;

    fn user(text: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"2026-11-08T10:00:00.000Z","message":{{"role":"user","content":{}}}}}"#,
            serde_json::to_string(text).unwrap()
        )
    }

    fn assistant(blocks: &str, extra: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-11-08T10:00:05.000Z"{extra},"message":{{"id":"m","content":[{blocks}]}}}}"#
        )
    }

    fn convo(lines: &[String]) -> Vec<Message> {
        conversation(&mut Cursor::new(lines.join("\n").into_bytes()))
    }

    #[test]
    fn turns_join_text_and_count_tools() {
        let m = convo(&[
            user("first <pasted_content id=1>x</pasted_content> question"),
            assistant(r#"{"type":"text","text":"Let me look."}"#, ""),
            assistant(r#"{"type":"tool_use","id":"t1","name":"Read","input":{}}"#, ""),
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"file body"}]}}"#
                .to_string(),
            assistant(
                r#"{"type":"tool_use","id":"t2","name":"Bash","input":{}},{"type":"text","text":"Done."}"#,
                "",
            ),
            user("/model opus"),
            user("second"),
            assistant(r#"{"type":"text","text":"Sure."}"#, ""),
        ]);
        let got: Vec<(&str, &str, u32)> = m
            .iter()
            .map(|x| (x.role, x.text.as_str(), x.tools))
            .collect();
        assert_eq!(
            got,
            [
                ("user", "first x question", 0),
                ("assistant", "Let me look.\n\nDone.", 2),
                ("user", "second", 0),
                ("assistant", "Sure.", 0),
            ]
        );
        assert!(m[0].ts.is_some() && m[1].ts.is_some());
    }

    #[test]
    fn leaves_out_sidechain_meta_and_injected_lines() {
        let m = convo(&[
            user("real"),
            assistant(
                r#"{"type":"text","text":"sub-agent talk"}"#,
                r#","isSidechain":true"#,
            ),
            r#"{"type":"user","isMeta":true,"message":{"content":"Caveat: injected"}}"#.to_string(),
            r#"{"type":"user","isSidechain":true,"message":{"content":"sub prompt"}}"#.to_string(),
            user("<system-reminder>only a reminder</system-reminder>"),
            r#"{"type":"attachment","x":"user"}"#.to_string(),
            assistant(r#"{"type":"text","text":"answer"}"#, ""),
        ]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].text, "answer");
    }

    #[test]
    fn masks_secrets_and_caps_text() {
        let long = "x".repeat(TEXT_CAP + 10);
        let m = convo(&[
            user("use password: hunter2hunter2 please"),
            assistant(&format!(r#"{{"type":"text","text":"{long}"}}"#), ""),
        ]);
        assert!(m[0].text.contains("[masked]") && !m[0].text.contains("hunter2"));
        assert!(!m[0].truncated);
        assert!(m[1].truncated);
        assert_eq!(m[1].text.chars().count(), TEXT_CAP);
    }
}
