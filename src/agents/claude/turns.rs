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

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/claude-turns.jsonl");

    fn feed(turns: &mut Turns, text: &str) {
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            let ty = v["type"].as_str().unwrap().to_string();
            turns.line(&v, &ty);
        }
    }

    fn kinds(events: &[TurnEvent]) -> Vec<(TurnKind, i64)> {
        events.iter().map(|e| (e.kind, e.n)).collect()
    }

    #[test]
    fn a_whole_transcript_reports_every_kind_once() {
        let mut t = Turns::resume(None);
        feed(&mut t, FIXTURE);
        let (events, carry) = t.finish(true);
        use TurnKind::*;
        assert_eq!(
            kinds(&events),
            vec![
                (Prompt, 20),
                (Think, 13),
                (Tool, 0),
                (ToolError, 0),
                (Tool, 0),
                (Refusal, 0),
                (Interrupt, 0),
                (End, 5),
                (Prompt, 23),
                (Compact, 0),
                (End, 8),
            ]
        );
        // the sub-agent's tool call, the command wrapper and the harness note are not turns
        assert_eq!(carry.unwrap()["closed"], true);
    }

    #[test]
    fn an_end_carries_the_time_of_the_last_reply_of_its_turn() {
        let mut t = Turns::resume(None);
        feed(&mut t, FIXTURE);
        let (events, _) = t.finish(true);
        let ends: Vec<i64> = events
            .iter()
            .filter(|e| e.kind == TurnKind::End)
            .map(|e| e.ts)
            .collect();
        let at = |m: u32, s: u32| iso_to_ms(&format!("2026-11-02T10:{m:02}:{s:02}.000Z")).unwrap();
        assert_eq!(ends, vec![at(2, 10), at(7, 0)]);
    }

    #[test]
    fn a_read_that_stops_inside_a_turn_does_not_close_it_twice() {
        let lines: Vec<&str> = FIXTURE.lines().collect();
        // the first read stops after the second prompt's first reply, the second goes on from there
        let (head, tail) = lines.split_at(13);
        let mut first = Turns::resume(None);
        feed(&mut first, &head.join("\n"));
        let (a, carry) = first.finish(false);
        assert_eq!(a.iter().filter(|e| e.kind == TurnKind::End).count(), 1);
        let mut second = Turns::resume(carry.as_ref());
        feed(&mut second, &tail.join("\n"));
        let (b, _) = second.finish(true);
        let ends = |e: &[TurnEvent]| e.iter().filter(|e| e.kind == TurnKind::End).count();
        assert_eq!(ends(&b), 1);
        let (_, n) = kinds(&b)
            .into_iter()
            .find(|(k, _)| *k == TurnKind::End)
            .unwrap();
        assert_eq!(n, 8);
    }

    #[test]
    fn a_turn_a_whole_read_closed_is_not_closed_again_by_the_next_read() {
        let mut first = Turns::resume(None);
        feed(
            &mut first,
            FIXTURE
                .lines()
                .take(14)
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        );
        let (a, carry) = first.finish(true);
        assert_eq!(a.last().map(|e| e.kind), Some(TurnKind::End));
        let mut second = Turns::resume(carry.as_ref());
        feed(
            &mut second,
            FIXTURE
                .lines()
                .skip(14)
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        );
        let (b, _) = second.finish(false);
        assert!(b.iter().all(|e| e.kind != TurnKind::End));
    }

    #[test]
    fn what_makes_a_failed_result_a_refusal() {
        let mut t = Turns::resume(None);
        for (text, want) in [
            ("Permission to use Bash has been denied.", TurnKind::Refusal),
            ("The tool use was REJECTED by a rule", TurnKind::Refusal),
            ("Exit code 1", TurnKind::ToolError),
        ] {
            let v = serde_json::json!({"type":"user","timestamp":"2026-11-02T10:00:00.000Z",
                "message":{"content":[{"type":"tool_result","is_error":true,"content":text}]}});
            t.line(&v, "user");
            assert_eq!(t.events.last().map(|e| e.kind), Some(want), "{text}");
        }
        // an error that only mentions it past the first 400 characters is a plain error
        let long = format!("{} permission", "x".repeat(400));
        let v = serde_json::json!({"type":"user","timestamp":"2026-11-02T10:00:00.000Z",
            "message":{"content":[{"type":"tool_result","is_error":true,"content":long}]}});
        t.line(&v, "user");
        assert_eq!(t.events.last().map(|e| e.kind), Some(TurnKind::ToolError));
    }

    #[test]
    fn a_result_that_did_not_fail_and_an_unsigned_thinking_block_are_nothing() {
        let mut t = Turns::resume(None);
        let ok = serde_json::json!({"type":"user","timestamp":"2026-11-02T10:00:00.000Z",
            "message":{"content":[{"type":"tool_result","content":"fine"}]}});
        t.line(&ok, "user");
        let think = serde_json::json!({"type":"assistant","timestamp":"2026-11-02T10:00:01.000Z",
            "message":{"content":[{"type":"thinking","thinking":"hm","signature":""}]}});
        t.line(&think, "assistant");
        assert!(t.events.is_empty());
    }

    #[test]
    fn an_empty_transcript_has_no_turns_and_nothing_to_carry() {
        let (events, carry) = Turns::resume(None).finish(true);
        assert!(events.is_empty() && carry.is_none());
    }

    #[test]
    fn the_whole_file_reader_matches_the_streaming_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        std::fs::write(&p, FIXTURE).unwrap();
        let mut t = Turns::resume(None);
        feed(&mut t, FIXTURE);
        assert_eq!(read_events(&p).unwrap(), t.finish(true).0);
    }
}
