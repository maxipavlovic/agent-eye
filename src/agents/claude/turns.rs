//! The turn events of a Claude Code transcript, for the steering clock (`crate::turns`).
//!
//! A turn ends with the last assistant line before the next prompt, which is only known once that
//! prompt arrives. The read that sees the prompt emits the `End`, so a read that stops in the
//! middle of a turn keeps the open turn (time of its last assistant line, words so far) in the
//! carry instead of emitting anything; the next read picks it up and the turn is closed once.
//! A read of the whole transcript also closes the last turn at its end, as nothing more is known
//! about it, and marks it closed in the carry, so a continuation of that same turn is not closed a
//! second time. The carry is cheaper than finding the start of the open turn again from the file.

use super::super::append_only::for_each_line;
use super::transcript::{line_type, result_text};
use crate::turns::{TurnEvent, TurnKind};
use crate::util::iso_to_ms;
use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::LazyLock;

/// How much of a failed tool result is looked at to tell a refusal from an error.
const RESULT_LOOK: usize = 400;

static RE_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)denied|rejected|doesn't want|not allowed|permission").expect("static regex")
});

/// The agent's turn that has not been closed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    /// Time of its last assistant line.
    ts: i64,
    /// Words of reply text so far.
    words: i64,
    /// Its `End` has been emitted (a whole read stopped inside it).
    closed: bool,
}

/// Collects the turn events of the lines it is shown, in file order.
pub(super) struct Turns {
    events: Vec<TurnEvent>,
    open: Option<Open>,
}

impl Turns {
    /// Start from the open turn an earlier read left in its carry (None: nothing is open).
    pub(super) fn resume(carry: Option<&Value>) -> Turns {
        let open = carry.and_then(|c| c.get("turn")).and_then(|t| {
            Some(Open {
                ts: t.get("ts")?.as_i64()?,
                words: t.get("words")?.as_i64()?,
                closed: t.get("closed")?.as_bool()?,
            })
        });
        Turns {
            events: vec![],
            open,
        }
    }

    /// Take one transcript line of type `ty`. Sub-agent lines and lines without a time are not
    /// part of the session's turns.
    pub(super) fn line(&mut self, v: &Value, ty: &str) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let Some(ts) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms)
        else {
            return;
        };
        match ty {
            "user" => self.user(v, ts),
            "assistant" => self.assistant(v, ts),
            "system" if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                self.push(ts, TurnKind::Compact, 0);
            }
            _ => {}
        }
    }

    fn push(&mut self, ts: i64, kind: TurnKind, n: i64) {
        self.events.push(TurnEvent::new(ts, kind, n));
    }

    fn user(&mut self, v: &Value, ts: i64) {
        let flag = |k| v.get(k).and_then(Value::as_bool) == Some(true);
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            return;
        };
        match content {
            Value::String(t) if is_interrupt(t) => self.push(ts, TurnKind::Interrupt, 0),
            Value::String(t) => {
                if flag("isMeta") || flag("isCompactSummary") || t.trim_start().starts_with('<') {
                    return;
                }
                self.close_open();
                self.push(ts, TurnKind::Prompt, t.chars().count() as i64);
            }
            Value::Array(blocks) => {
                for b in blocks {
                    match b.get("type").and_then(Value::as_str) {
                        Some("text")
                            if b.get("text")
                                .and_then(Value::as_str)
                                .is_some_and(is_interrupt) =>
                        {
                            self.push(ts, TurnKind::Interrupt, 0);
                        }
                        Some("tool_result") => self.result(b, ts),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// A failed tool result is a refusal when its text says the user or a rule said no.
    fn result(&mut self, b: &Value, ts: i64) {
        if b.get("is_error").and_then(Value::as_bool) != Some(true) {
            return;
        }
        let text = result_text(b.get("content"));
        let look = text
            .char_indices()
            .nth(RESULT_LOOK)
            .map_or(&*text, |(i, _)| &text[..i]);
        let kind = if RE_REFUSAL.is_match(look) {
            TurnKind::Refusal
        } else {
            TurnKind::ToolError
        };
        self.push(ts, kind, 0);
    }

    fn assistant(&mut self, v: &Value, ts: i64) {
        let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) else {
            return;
        };
        let mut words = 0;
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    words += b
                        .get("text")
                        .and_then(Value::as_str)
                        .map_or(0, |t| t.split_whitespace().count() as i64);
                }
                Some("tool_use") => self.push(ts, TurnKind::Tool, 0),
                Some("thinking") => {
                    let sig = b.get("signature").and_then(Value::as_str).unwrap_or("");
                    if !sig.is_empty() {
                        self.push(ts, TurnKind::Think, sig.len() as i64);
                    }
                }
                _ => {}
            }
        }
        match &mut self.open {
            Some(o) if o.closed => {}
            Some(o) => {
                o.ts = ts;
                o.words += words;
            }
            None => {
                self.open = Some(Open {
                    ts,
                    words,
                    closed: false,
                });
            }
        }
    }

    /// Emit the `End` of the open turn, once.
    fn close_open(&mut self) {
        if let Some(o) = self.open.take().filter(|o| !o.closed) {
            self.push(o.ts, TurnKind::End, o.words);
        }
    }

    /// The events, and the turn to carry to the next read. A `whole` read closes the last turn.
    pub(super) fn finish(mut self, whole: bool) -> (Vec<TurnEvent>, Option<Value>) {
        if whole {
            if let Some(o) = self.open.as_mut().filter(|o| !o.closed) {
                let end = TurnEvent::new(o.ts, TurnKind::End, o.words);
                o.closed = true;
                self.events.push(end);
            }
        }
        let carry = self
            .open
            .map(|o| json!({"ts": o.ts, "words": o.words, "closed": o.closed}));
        (self.events, carry)
    }
}

fn is_interrupt(t: &str) -> bool {
    t.starts_with("[Request interrupted")
}

/// The events of the whole transcript at `path`; only the lines that can matter are parsed.
pub(super) fn read_events(path: &Path) -> Result<Vec<TurnEvent>> {
    let mut turns = Turns::resume(None);
    for_each_line(path, 0, |line| {
        if !matches!(
            line_type(line),
            None | Some("user" | "assistant" | "system")
        ) {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(ty) = v.get("type").and_then(Value::as_str) {
            turns.line(&v, ty);
        }
    })?;
    Ok(turns.finish(true).0)
}
