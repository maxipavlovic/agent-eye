//! The conversation of a session as a list of messages, read from its transcript (or from a copy
//! made from eye's archive when the transcript is gone).

use crate::agents::Agent;
use crate::archive;
use crate::model::Session;
use crate::paths::Paths;
use crate::summarize::mask_secrets;
use crate::util::head_chars;
use anyhow::Result;
use serde_json::Value;

/// Longest text of one message, in characters.
pub const TEXT_CAP: usize = 4_000;
/// Messages in a window when the caller gives no limit.
pub const DEFAULT_LIMIT: usize = 10;
/// Most messages in one window.
pub const MAX_LIMIT: usize = 50;

/// One message of a conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    /// "user" or "assistant".
    pub role: &'static str,
    pub text: String,
    pub ts: Option<i64>,
    /// Tool calls the agent made in this turn (assistant messages only).
    pub tools: u32,
    pub truncated: bool,
}

impl Message {
    /// A message with secrets masked and the text cut at [`TEXT_CAP`].
    pub fn new(role: &'static str, text: &str, ts: Option<i64>, tools: u32) -> Message {
        let masked = mask_secrets(text.trim());
        let truncated = masked.chars().count() > TEXT_CAP;
        Message {
            role,
            text: if truncated {
                head_chars(&masked, TEXT_CAP)
            } else {
                masked
            },
            ts,
            tools,
            truncated,
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "role": self.role, "text": self.text, "ts": self.ts,
            "tools": self.tools, "truncated": self.truncated,
        })
    }
}

/// A window of the conversation, oldest first.
#[derive(Debug, Clone)]
pub struct Page {
    pub messages: Vec<Message>,
    pub more: bool,
    /// Index to pass as `before` for the next older window.
    pub next_before: Option<usize>,
}

impl Page {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "messages": self.messages.iter().map(Message::to_json).collect::<Vec<_>>(),
            "more": self.more, "next_before": self.next_before,
        })
    }
}

/// The `limit` messages just before index `before` (default: the end), still oldest first.
pub fn page(all: &[Message], limit: usize, before: Option<usize>) -> Page {
    let end = before.unwrap_or(all.len()).min(all.len());
    let start = end.saturating_sub(limit);
    Page {
        messages: all[start..end].to_vec(),
        more: start > 0,
        next_before: (start > 0).then_some(start),
    }
}

/// Every message of the session, oldest first, read through the session's agent.
pub fn read(s: &Session, paths: &Paths, agent: &dyn Agent) -> Result<Vec<Message>> {
    let copy = archive::readable(s, paths, agent)?;
    agent.read_conversation(&copy.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_walk_back_from_the_newest() {
        let all: Vec<Message> = (0..7)
            .map(|i| Message::new("user", &i.to_string(), None, 0))
            .collect();
        let p = page(&all, 3, None);
        let t = |p: &Page| {
            p.messages
                .iter()
                .map(|m| m.text.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(t(&p), ["4", "5", "6"]);
        assert!(p.more);
        assert_eq!(p.next_before, Some(4));
        let p = page(&all, 3, p.next_before);
        assert_eq!(t(&p), ["1", "2", "3"]);
        let p = page(&all, 3, p.next_before);
        assert_eq!(t(&p), ["0"]);
        assert!(!p.more && p.next_before.is_none());
        assert!(page(&all, 3, Some(99)).messages.len() == 3);
        assert!(page(&[], 3, None).messages.is_empty());
    }

    #[test]
    fn text_is_masked_and_capped() {
        let m = Message::new("user", "use password: hunter2hunter2 please", None, 0);
        assert!(m.text.contains("[masked]") && !m.text.contains("hunter2"));
        assert!(!m.truncated);
        let long = Message::new("assistant", &"x".repeat(TEXT_CAP + 10), None, 2);
        assert!(long.truncated);
        assert_eq!(long.text.chars().count(), TEXT_CAP);
    }

    #[test]
    fn reads_the_archive_copy_when_the_transcript_is_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        let claude = crate::agents::claude::Claude::new(tmp.path().join("claude"));
        let body = [
            r#"{"type":"user","timestamp":"2026-11-08T10:00:00.000Z","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-08T10:00:05.000Z","message":{"id":"m","content":[{"type":"text","text":"hi"}]}}"#,
        ]
        .join("\n");
        let arch = archive::archive_path(&paths, &claude, "-tmp-x", "abc");
        std::fs::create_dir_all(arch.parent().unwrap()).unwrap();
        std::fs::write(&arch, zstd::encode_all(body.as_bytes(), 3).unwrap()).unwrap();
        let s = Session {
            id: "abc".into(),
            project_dir: Some("-tmp-x".into()),
            transcript_path: Some(tmp.path().join("gone.jsonl").to_string_lossy().into()),
            ..Default::default()
        };
        let m = read(&s, &paths, &claude).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].text, "hi");
        let none = Session {
            id: "zzz".into(),
            ..Default::default()
        };
        assert!(read(&none, &paths, &claude).is_err());
    }
}
