//! The turn events of an agy log, for the steering clock (`crate::turns`).
//!
//! A turn ends with the last model reply before the next user step (see `crate::turns::stream`).
//! The log keeps no token count for the thinking of a call (`output_tokens` has it inside the
//! reply), so a thought is the length of its `thinking` text. A failed tool result is `ERROR`, or
//! `INVALID` for a call the model wrote wrongly; only an `ERROR` that says a permission check
//! failed or the user denied the call is a refusal. The log has no step for a user's interrupt or
//! a compaction.

use super::log::{prompt_text, Step};
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

static RE_REFUSAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)permission check failed|user denied").expect("static regex"));

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
                let refused = s.status == "ERROR"
                    && s.error.as_deref().is_some_and(|e| RE_REFUSAL.is_match(e));
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

    /// The events, and the turn to carry to the next read.
    pub(super) fn finish(self) -> (Vec<TurnEvent>, Option<Value>) {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::agy::log::decode;
    use crate::turns::testing::{end_times, kinds, read_in_two, wrapped};
    use crate::util::iso_to_ms;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/agy-turns.jsonl");
    /// A prompt that follows the fixture's last reply.
    const NEXT_PROMPT: &str = r#"{"step_index":13,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-04T08:09:00Z","content":"<USER_REQUEST>\nship it\n</USER_REQUEST>"}"#;

    fn feed(turns: &mut Turns, text: &str) {
        for line in text.lines() {
            turns.step(&decode(line.as_bytes()).unwrap());
        }
    }

    fn read(carry: Option<&Value>, text: &str) -> (Vec<TurnEvent>, Option<Value>) {
        let carry = carry.and_then(|c| wrapped(&Some(c.clone())));
        let mut t = Turns::resume(carry.as_ref());
        feed(&mut t, text);
        t.finish()
    }

    fn at(m: u32, s: u32) -> i64 {
        iso_to_ms(&format!("2026-11-04T08:{m:02}:{s:02}Z")).unwrap()
    }

    fn with_next_prompt() -> String {
        format!("{}\n{NEXT_PROMPT}", FIXTURE.trim_end())
    }

    fn step(status: &str, error: &str) -> Step {
        let line = serde_json::json!({"type": "GENERIC", "status": status, "error": error,
            "created_at": "2026-11-04T08:00:00Z"});
        decode(line.to_string().as_bytes()).unwrap()
    }

    #[test]
    fn a_whole_log_reports_every_kind_it_records_and_leaves_its_last_turn_open() {
        let (events, carry) = read(None, FIXTURE);
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
            ]
        );
        // an injected system message and a model error are not turns
        let open = carry.unwrap();
        assert_eq!(
            (open["words"].as_i64(), open["ts"].as_i64()),
            (Some(2), Some(at(6, 0)))
        );
    }

    #[test]
    fn an_end_carries_the_time_of_the_last_reply_of_its_turn() {
        let (events, _) = read(None, &with_next_prompt());
        assert_eq!(end_times(&events), vec![at(0, 20), at(6, 0)]);
    }

    #[test]
    fn a_turn_read_in_two_goes_ends_once_at_its_last_reply() {
        // the first read stops after the second prompt's first reply
        let (a, b) = read_in_two(&with_next_prompt(), 11, read);
        assert_eq!(end_times(&a), vec![at(0, 20)]);
        assert_eq!(end_times(&b), vec![at(6, 0)]);
        assert_eq!(kinds(&b).last(), Some(&(TurnKind::Prompt, 7)));
        assert_eq!(
            b.iter().find(|e| e.kind == TurnKind::End).map(|e| e.n),
            Some(2)
        );
    }

    #[test]
    fn a_turn_open_at_the_end_of_a_read_is_not_closed_by_the_next_read_that_goes_on_in_it() {
        let (a, b) = read_in_two(FIXTURE, 8, read);
        assert!(end_times(&a).is_empty());
        assert_eq!(end_times(&b), vec![at(0, 20)]);
    }

    #[test]
    fn only_a_failed_permission_check_or_a_denial_is_a_refusal() {
        let mut t = Turns::resume(None);
        for (status, error, want) in [
            (
                "ERROR",
                "permission check failed for unsandboxed \"x\"",
                TurnKind::Refusal,
            ),
            ("ERROR", "user denied the command", TurnKind::Refusal),
            ("ERROR", "exit status 2", TurnKind::ToolError),
            ("INVALID", "invalid arguments", TurnKind::ToolError),
            // the model wrote the call wrongly: no one refused it
            (
                "INVALID",
                "invalid arguments: - additional properties 'topn' not allowed",
                TurnKind::ToolError,
            ),
            ("INVALID", "permission check failed", TurnKind::ToolError),
        ] {
            t.step(&step(status, error));
            assert_eq!(t.0.last().map(|e| e.kind), Some(want), "{error}");
        }
        let ok = serde_json::json!({"type": "GENERIC", "status": "DONE", "created_at": "2026-11-04T08:00:00Z"});
        t.step(&decode(ok.to_string().as_bytes()).unwrap());
        let running = serde_json::json!({"type": "GENERIC", "status": "RUNNING", "created_at": "2026-11-04T08:00:00Z"});
        t.step(&decode(running.to_string().as_bytes()).unwrap());
        assert_eq!(t.finish().0.len(), 6);
    }

    #[test]
    fn an_empty_log_has_no_turns_and_nothing_to_carry() {
        let (events, carry) = Turns::resume(None).finish();
        assert!(events.is_empty() && carry.is_none());
    }
}
