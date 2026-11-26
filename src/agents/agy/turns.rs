//! The turn events of an agy log, for the steering clock (`crate::turns`).
//!
//! A turn ends with the last model reply before the next user step (see `crate::turns::stream`).
//! The log keeps no token count for the thinking of a call (`output_tokens` has it inside the
//! reply), so a thought is the length of its `thinking` text. A failed tool result (`ERROR`, or
//! `INVALID` for a call the model wrote wrongly) is a refusal when agy says a permission check
//! failed. The log has no step for a user's interrupt or a compaction.

use super::log::{prompt_text, Step};
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static RE_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)permission|denied|rejected|not allowed").expect("static regex")
});

/// Collects the turn events of the steps it is shown, in log order.
pub(super) struct Turns(Stream);

impl Turns {
    /// Start from the open turn an earlier read left in its carry (None: nothing is open).
    pub(super) fn resume(carry: Option<&Value>) -> Turns {
        Turns(Stream::resume(carry))
    }

    /// Take one step. A step without a time is not part of the turns.
    pub(super) fn step(&mut self, s: &Step) {
        let Some(ts) = s.ts else {
            return;
        };
        match s.kind.as_str() {
            "USER_INPUT" => {
                if let Some(p) = prompt_text(&s.content) {
                    self.0.close();
                    self.0.push(ts, TurnKind::Prompt, p.chars().count() as i64);
                }
            }
            "PLANNER_RESPONSE" => {
                let words = s.content.split_whitespace().count() as i64;
                self.0.reply(ts, words);
                if s.thinking > 0 {
                    self.0.push(ts, TurnKind::Think, s.thinking);
                }
                for _ in &s.tools {
                    self.0.push(ts, TurnKind::Tool, 0);
                }
            }
            "GENERIC" if matches!(s.status.as_str(), "ERROR" | "INVALID") => {
                let refused = s.error.as_deref().is_some_and(|e| RE_REFUSAL.is_match(e));
                let kind = if refused {
                    TurnKind::Refusal
                } else {
                    TurnKind::ToolError
                };
                self.0.push(ts, kind, 0);
            }
            _ => {}
        }
    }

    /// The events, and the turn to carry to the next read. A `whole` read closes the last turn.
    pub(super) fn finish(self, whole: bool) -> (Vec<TurnEvent>, Option<Value>) {
        self.0.finish(whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::agy::log::decode;
    use crate::util::iso_to_ms;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/agy-turns.jsonl");

    fn feed(turns: &mut Turns, text: &str) {
        for line in text.lines() {
            turns.step(&decode(line.as_bytes()).unwrap());
        }
    }

    /// The carry as an adapter stores it: under `turn`.
    fn wrapped(carry: &Option<Value>) -> Option<Value> {
        Some(serde_json::json!({ "turn": carry }))
    }

    fn kinds(events: &[TurnEvent]) -> Vec<(TurnKind, i64)> {
        events.iter().map(|e| (e.kind, e.n)).collect()
    }

    #[test]
    fn a_whole_log_reports_every_kind_it_records_once() {
        let mut t = Turns::resume(None);
        feed(&mut t, FIXTURE);
        let (events, carry) = t.finish(true);
        use TurnKind::*;
        assert_eq!(
            kinds(&events),
            vec![
                (Prompt, 20),
                (Think, 26),
                (Tool, 0),
                (ToolError, 0),
                (Tool, 0),
                (Refusal, 0),
                (End, 4),
                (Prompt, 9),
                (Tool, 0),
                (ToolError, 0),
                (End, 2),
            ]
        );
        // an injected system message and a model error are not turns
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
        let at = |m: u32, s: u32| iso_to_ms(&format!("2026-11-04T08:{m:02}:{s:02}Z")).unwrap();
        assert_eq!(ends, vec![at(0, 20), at(6, 0)]);
    }

    #[test]
    fn a_read_that_stops_inside_a_turn_does_not_close_it_twice() {
        let lines: Vec<&str> = FIXTURE.lines().collect();
        // the first read stops after the second prompt's first reply
        let (head, tail) = lines.split_at(11);
        let mut first = Turns::resume(None);
        feed(&mut first, &head.join("\n"));
        let (a, carry) = first.finish(false);
        assert_eq!(a.iter().filter(|e| e.kind == TurnKind::End).count(), 1);
        let mut second = Turns::resume(wrapped(&carry).as_ref());
        feed(&mut second, &tail.join("\n"));
        let (b, _) = second.finish(true);
        assert_eq!(
            kinds(&b)
                .iter()
                .filter(|(k, _)| *k == TurnKind::End)
                .count(),
            1
        );
        assert_eq!(kinds(&b).last(), Some(&(TurnKind::End, 2)));
    }

    #[test]
    fn a_turn_a_whole_read_closed_is_not_closed_again_by_the_next_read() {
        let lines: Vec<&str> = FIXTURE.lines().collect();
        let (head, tail) = lines.split_at(8);
        let mut first = Turns::resume(None);
        feed(&mut first, &head.join("\n"));
        let (a, carry) = first.finish(true);
        assert_eq!(a.last().map(|e| e.kind), Some(TurnKind::End));
        let mut second = Turns::resume(wrapped(&carry).as_ref());
        feed(&mut second, &tail.join("\n"));
        let (b, _) = second.finish(false);
        assert_eq!(
            kinds(&b)
                .iter()
                .filter(|(k, _)| *k == TurnKind::End)
                .count(),
            1
        );
    }

    #[test]
    fn only_a_failed_permission_check_is_a_refusal() {
        let mut t = Turns::resume(None);
        for (status, error, want) in [
            (
                "ERROR",
                "permission check failed for unsandboxed \"x\"",
                TurnKind::Refusal,
            ),
            ("ERROR", "exit status 2", TurnKind::ToolError),
            ("INVALID", "invalid arguments", TurnKind::ToolError),
        ] {
            let line = serde_json::json!({"type": "GENERIC", "status": status, "error": error,
                "created_at": "2026-11-04T08:00:00Z"});
            t.step(&decode(line.to_string().as_bytes()).unwrap());
            assert_eq!(t.0.last().map(|e| e.kind), Some(want), "{error}");
        }
        let ok = serde_json::json!({"type": "GENERIC", "status": "DONE", "created_at": "2026-11-04T08:00:00Z"});
        t.step(&decode(ok.to_string().as_bytes()).unwrap());
        let running = serde_json::json!({"type": "GENERIC", "status": "RUNNING", "created_at": "2026-11-04T08:00:00Z"});
        t.step(&decode(running.to_string().as_bytes()).unwrap());
        assert_eq!(t.finish(false).0.len(), 3);
    }

    #[test]
    fn an_empty_log_has_no_turns_and_nothing_to_carry() {
        let (events, carry) = Turns::resume(None).finish(true);
        assert!(events.is_empty() && carry.is_none());
    }
}
