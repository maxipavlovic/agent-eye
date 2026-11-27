//! The turn events of a Codex rollout, for the steering clock (`crate::turns`).
//!
//! Codex says itself when a turn is done (`task_complete`), which closes it there; a turn the user
//! interrupted has no such line, so it ends with its last reply, when the next prompt arrives (see
//! `crate::turns::stream`). The thinking of a turn is read from the thread's token totals: the
//! growth of `reasoning_output_tokens` since the last total, so a total that is written twice
//! counts once. Newer rollouts record a refused call as a tool item whose status is `declined`;
//! older ones hand the model "exec command rejected by user" (or "patch ...") as the call's output.
//! A tool item whose status is `failed` is a tool error, as is a command that exits non-zero.

use super::rollout::{exit_code, read_line, Event};
use crate::agents::append_only::for_each_line;
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use crate::util::iso_to_ms;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

/// `compacted` and `context_compacted` both announce one compaction; two within this many
/// milliseconds are one.
const SAME_COMPACTION_MS: i64 = 5_000;

/// What the output of a refused call of an older rollout starts with.
const REJECTED: [&str; 2] = ["exec command rejected by user", "patch rejected by user"];

/// Collects the turn events of the lines it is shown, in file order.
pub(super) struct Turns {
    stream: Stream,
    /// The thread's reasoning tokens at the last total.
    reasoning: i64,
    /// When the last compaction was seen, which may have been in an earlier read.
    compacted_at: Option<i64>,
}

impl Turns {
    /// Start from what an earlier read left in its carry (None: a first read).
    pub(super) fn resume(carry: Option<&Value>) -> Turns {
        Turns {
            stream: Stream::resume(carry),
            reasoning: carry
                .and_then(|c| c.get("reasoning"))
                .and_then(Value::as_i64)
                .unwrap_or(0),
            compacted_at: carry
                .and_then(|c| c.get("compacted_at"))
                .and_then(Value::as_i64),
        }
    }

    /// Take one decoded line of the rollout, `ts` its time (Unix ms).
    pub(super) fn line(&mut self, v: &Value, event: &Event, ts: i64) {
        let s = &mut self.stream;
        match event {
            Event::Prompt(p) => {
                s.close();
                s.push(ts, TurnKind::Prompt, p.chars().count() as i64);
            }
            Event::Reply(t) => s.reply(ts, t.split_whitespace().count() as i64),
            Event::Call { .. } => {
                s.reply(ts, 0);
                s.push(ts, TurnKind::Tool, 0);
            }
            Event::Output { text, .. } => {
                if REJECTED.iter().any(|r| text.trim_start().starts_with(r)) {
                    s.push(ts, TurnKind::Refusal, 0);
                } else if exit_code(text).is_some_and(|c| c != 0) {
                    s.push(ts, TurnKind::ToolError, 0);
                }
            }
            Event::Done { exit, .. } => {
                s.reply(ts, 0);
                s.push(ts, TurnKind::Tool, 0);
                if item_status(v) == Some("declined") {
                    s.push(ts, TurnKind::Refusal, 0);
                } else if item_status(v) == Some("failed") || exit.is_some_and(|c| c != 0) {
                    s.push(ts, TurnKind::ToolError, 0);
                }
            }
            Event::Total(_) => self.total(v, ts),
            Event::Compacted => {
                let again = self
                    .compacted_at
                    .is_some_and(|t| ts - t <= SAME_COMPACTION_MS);
                if !again {
                    s.push(ts, TurnKind::Compact, 0);
                    self.compacted_at = Some(ts);
                }
            }
            Event::Meta { .. } | Event::Turn { .. } | Event::Other => match payload_type(v) {
                Some("task_complete") => s.close_at(ts),
                // a task that a newer one replaced, or that ran out of budget, is no one's interrupt
                Some("turn_aborted")
                    if v.pointer("/payload/reason").and_then(Value::as_str)
                        == Some("interrupted") =>
                {
                    s.push(ts, TurnKind::Interrupt, 0);
                    s.abandon();
                }
                _ => {}
            },
        }
    }

    /// A token total: what the thread reasoned since the last one is one thought. A total that
    /// went down (a rollout that was rewritten) is only a new baseline.
    fn total(&mut self, v: &Value, ts: i64) {
        let Some(now) = [
            "/payload/info/total_token_usage",
            "/payload/thread_token_usage",
        ]
        .iter()
        .find_map(|p| v.pointer(p))
        .and_then(|u| u.get("reasoning_output_tokens"))
        .and_then(Value::as_i64) else {
            return;
        };
        if now > self.reasoning {
            self.stream.push(ts, TurnKind::Think, now - self.reasoning);
        }
        self.reasoning = now;
    }

    /// The events, and what to carry to the next read.
    pub(super) fn finish(self) -> (Vec<TurnEvent>, Option<Value>) {
        let reasoning = self.reasoning;
        let compacted_at = self.compacted_at;
        let (events, open) = self.stream.finish();
        let carry = json!({"turn": open, "reasoning": reasoning, "compacted_at": compacted_at});
        (events, Some(carry))
    }
}

fn payload_type(v: &Value) -> Option<&str> {
    v.pointer("/payload/type").and_then(Value::as_str)
}

/// The status of the item of an `item_completed` line.
fn item_status(v: &Value) -> Option<&str> {
    v.pointer("/payload/item/status").and_then(Value::as_str)
}

/// The events of the whole rollout at `path`.
pub(super) fn read_events(path: &Path) -> Result<Vec<TurnEvent>> {
    let mut turns = Turns::resume(None);
    for_each_line(path, 0, |line| {
        let Some((v, event)) = read_line(line) else {
            return;
        };
        if let Some(ts) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms)
        {
            turns.line(&v, &event, ts);
        }
    })?;
    Ok(turns.finish().0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::codex::rollout::decode;
    use crate::turns::testing::{assert_every_split_matches_whole, end_times, kinds, read_in_two};

    const FIXTURE: &str = include_str!("../../../tests/fixtures/codex-turns.jsonl");

    fn feed(turns: &mut Turns, text: &str) {
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            let ts = iso_to_ms(v["timestamp"].as_str().unwrap()).unwrap();
            turns.line(&v, &decode(&v), ts);
        }
    }

    fn read(carry: Option<&Value>, text: &str) -> (Vec<TurnEvent>, Option<Value>) {
        let mut t = Turns::resume(carry);
        feed(&mut t, text);
        t.finish()
    }

    fn at(m: u32, s: u32) -> i64 {
        iso_to_ms(&format!("2026-11-03T09:{m:02}:{s:02}.000Z")).unwrap()
    }

    /// The kinds the lines make, one line each, at the same time.
    fn of(lines: &[Value]) -> Vec<TurnKind> {
        let mut t = Turns::resume(None);
        for v in lines {
            t.line(v, &decode(v), 1);
        }
        t.finish().0.iter().map(|e| e.kind).collect()
    }

    #[test]
    fn a_whole_rollout_reports_every_kind_once() {
        let (events, carry) = read(None, FIXTURE);
        use TurnKind::*;
        assert_eq!(
            kinds(&events),
            vec![
                (Prompt, 20),
                (Tool, 0),
                (Think, 40),
                (Tool, 0),
                (ToolError, 0),
                (Tool, 0),
                (Refusal, 0),
                (Interrupt, 0),
                (End, 4),
                (Prompt, 9),
                (Compact, 0),
                (End, 2),
                // `/compact` is a task nobody typed: a compaction and no end
                (Compact, 0),
                (Prompt, 12),
                // the total and the record that repeats it are one thought
                (Think, 15),
                // a turn the model failed to answer still ends, with no words
                (End, 0),
            ]
        );
        assert_eq!(carry.unwrap()["reasoning"], 55);
    }

    #[test]
    fn an_interrupted_turn_ends_with_its_last_reply_and_a_finished_one_at_task_complete() {
        let (events, _) = read(None, FIXTURE);
        assert_eq!(end_times(&events), vec![at(0, 20), at(6, 2), at(8, 9)]);
    }

    #[test]
    fn a_read_that_stops_inside_a_turn_does_not_close_it_twice() {
        // the first read stops after the second prompt, the second goes on from there
        let (a, b) = read_in_two(FIXTURE, 12, read);
        assert_eq!(end_times(&a), vec![at(0, 20)]);
        assert_eq!(end_times(&b), vec![at(6, 2), at(8, 9)]);
    }

    #[test]
    fn a_first_read_in_the_middle_of_a_turn_does_not_end_it_before_task_complete() {
        // the first read has the last reply and not yet the task_complete line after it
        let (a, b) = read_in_two(FIXTURE, 15, read);
        assert_eq!(end_times(&a), vec![at(0, 20)]);
        assert_eq!(end_times(&b), vec![at(6, 2), at(8, 9)]);
        assert_eq!(
            b.iter().find(|e| e.kind == TurnKind::End).map(|e| e.n),
            Some(2)
        );
    }

    #[test]
    fn every_split_point_gives_the_events_of_a_whole_read() {
        assert_every_split_matches_whole(FIXTURE, read);
    }

    #[test]
    fn a_task_nobody_prompted_adds_no_end_to_the_turn_before_it() {
        let (events, _) = read(None, FIXTURE);
        let ends = end_times(&events);
        // the `/compact` task finishes at 09:07:03 and is not the end of any turn
        assert!(!ends.contains(&at(7, 3)));
        assert_eq!(ends.len(), 3);
    }

    #[test]
    fn a_task_that_finishes_after_an_interrupt_does_not_end_the_interrupted_turn() {
        let lines = [
            json!({"timestamp": "2026-11-03T09:00:00.000Z", "type": "event_msg", "payload":
                {"type": "item_completed", "item": {"type": "UserMessage", "id": "u", "content": [{"type": "text", "text": "go"}]}}}),
            json!({"timestamp": "2026-11-03T09:00:05.000Z", "type": "event_msg", "payload":
                {"type": "item_completed", "item": {"type": "AgentMessage", "id": "a", "content": [{"type": "text", "text": "one two"}]}}}),
            json!({"timestamp": "2026-11-03T09:00:09.000Z", "type": "event_msg", "payload":
                {"type": "turn_aborted", "reason": "interrupted"}}),
            json!({"timestamp": "2026-11-03T09:05:00.000Z", "type": "event_msg", "payload":
                {"type": "task_complete", "turn_id": "review"}}),
        ];
        let mut t = Turns::resume(None);
        for v in &lines {
            let ts = iso_to_ms(v["timestamp"].as_str().unwrap()).unwrap();
            t.line(v, &decode(v), ts);
        }
        let (events, carry) = t.finish();
        assert!(end_times(&events).is_empty());
        // the turn stays open and ends with its last reply at the next prompt
        assert_eq!(carry.unwrap()["turn"]["words"], 2);
    }

    #[test]
    fn a_compaction_whose_two_lines_fall_in_two_reads_is_stored_once() {
        let line = |ts: &str, ty: &str| match ty {
            "compacted" => {
                json!({"timestamp": ts, "type": "compacted", "payload": {"message": ""}})
            }
            _ => json!({"timestamp": ts, "type": "event_msg", "payload": {"type": ty}}),
        };
        let first = line("2026-11-03T09:05:10.000Z", "compacted").to_string();
        let second = line("2026-11-03T09:05:11.000Z", "context_compacted").to_string();
        let (a, carry) = read(None, &first);
        let (b, _) = read(carry.as_ref(), &second);
        assert_eq!(kinds(&a), vec![(TurnKind::Compact, 0)]);
        assert!(b.is_empty());
    }

    #[test]
    fn only_the_user_interrupt_reason_of_turn_aborted_is_an_interrupt() {
        let aborted = |reason: &str| {
            json!({"timestamp": "2026-11-03T09:00:25.000Z", "type": "event_msg",
                "payload": {"type": "turn_aborted", "reason": reason}})
        };
        let lines = [
            aborted("interrupted"),
            aborted("replaced"),
            aborted("budget_limited"),
        ];
        assert_eq!(of(&lines), vec![TurnKind::Interrupt]);
    }

    #[test]
    fn reasoning_counts_what_the_total_grew_by_and_a_repeated_total_counts_nothing() {
        let mut t = Turns::resume(None);
        let total = |ts: &str, r: i64| {
            json!({"timestamp": ts, "type": "event_msg", "payload": {"type": "token_count",
                "info": {"total_token_usage": {"reasoning_output_tokens": r}}}})
        };
        for (ts, r) in [
            ("2026-11-03T09:00:01.000Z", 30),
            ("2026-11-03T09:00:02.000Z", 30),
            ("2026-11-03T09:00:03.000Z", 50),
            // a total that went down is a new baseline, not a thought
            ("2026-11-03T09:00:04.000Z", 10),
            ("2026-11-03T09:00:05.000Z", 15),
        ] {
            let v = total(ts, r);
            t.line(&v, &decode(&v), iso_to_ms(ts).unwrap());
        }
        let (events, _) = t.finish();
        assert_eq!(
            kinds(&events),
            vec![
                (TurnKind::Think, 30),
                (TurnKind::Think, 20),
                (TurnKind::Think, 5)
            ]
        );
    }

    #[test]
    fn a_legacy_call_and_its_failed_output_are_a_tool_and_a_tool_error() {
        let call = |id: &str| {
            json!({"timestamp": "2026-11-03T09:00:01.000Z", "type": "response_item", "payload":
                {"type": "function_call", "name": "shell_command", "call_id": id, "arguments": "{\"command\":\"false\"}"}})
        };
        let output = |id: &str, text: &str| {
            json!({"timestamp": "2026-11-03T09:00:02.000Z", "type": "response_item", "payload":
                {"type": "function_call_output", "call_id": id, "output": text}})
        };
        let lines = [
            call("c1"),
            output("c1", "Process exited with code 1\nOutput:\n"),
            output("c2", "Process exited with code 0\nOutput:\n"),
        ];
        use TurnKind::*;
        assert_eq!(of(&lines), vec![Tool, ToolError]);
    }

    #[test]
    fn a_call_an_older_rollout_says_the_user_rejected_is_a_refusal() {
        let output = |text: &str| {
            json!({"timestamp": "2026-11-03T09:00:02.000Z", "type": "response_item", "payload":
                {"type": "function_call_output", "call_id": "c", "output": text}})
        };
        let lines = [
            output("exec command rejected by user"),
            output("patch rejected by user"),
            output("the rejected patch is not mentioned first"),
        ];
        assert_eq!(of(&lines), vec![TurnKind::Refusal, TurnKind::Refusal]);
        // a failed command that says "rejected" or "permission denied" is not one
        let failures: Vec<Value> = crate::turns::testing::NOT_REFUSALS
            .iter()
            .map(|t| output(&format!("Process exited with code 1\nOutput:\n{t}")))
            .collect();
        assert!(!of(&failures).contains(&TurnKind::Refusal));
    }

    #[test]
    fn a_failed_patch_or_mcp_call_is_a_tool_error_and_a_completed_one_is_not() {
        let item = |kind: &str, status: &str| {
            json!({"timestamp": "2026-11-03T09:00:02.000Z", "type": "event_msg", "payload":
                {"type": "item_completed", "item": {"type": kind, "id": "i", "status": status,
                    "server": "docs", "tool": "search", "changes": {}}}})
        };
        let lines = [
            item("FileChange", "failed"),
            item("McpToolCall", "failed"),
            item("McpToolCall", "completed"),
        ];
        use TurnKind::*;
        assert_eq!(of(&lines), vec![Tool, ToolError, Tool, ToolError, Tool]);
    }

    #[test]
    fn an_empty_rollout_has_no_turns() {
        let (events, carry) = Turns::resume(None).finish();
        assert!(events.is_empty());
        assert_eq!(carry.unwrap()["turn"], Value::Null);
    }

    #[test]
    fn the_whole_file_reader_matches_the_streaming_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rollout.jsonl");
        std::fs::write(&p, FIXTURE).unwrap();
        assert_eq!(read_events(&p).unwrap(), read(None, FIXTURE).0);
    }
}
