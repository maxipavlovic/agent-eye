//! The turn events of a Claude Code transcript, for the steering clock (`crate::turns`). A turn
//! ends with the last assistant line before the next prompt (see `crate::turns::stream`).

use super::super::append_only::for_each_line;
use super::prompt::typed_prompt;
use super::transcript::{is_queued_prompt, line_type, result_text};
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use crate::util::iso_to_ms;
use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::path::Path;
use std::sync::LazyLock;

/// How Claude Code words a permission decision in a tool result: the user said no ("The user
/// doesn't want to proceed with this tool use", "... to take this action right now"), or a rule or
/// the permission mode did ("Permission to use Bash has been denied", "Permission for this action
/// ..."). A failed command that happens to say "Permission denied" or "rejected" is a tool error,
/// not a decision.
static RE_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(The user doesn't want to|Permission (to use|for this))")
        .expect("static regex")
});

/// Collects the turn events of the lines it is shown, in file order.
pub(super) struct Turns(Stream);

impl Turns {
    /// Start from the open turn an earlier read left in its carry (None: nothing is open).
    pub(super) fn resume(carry: Option<&Value>) -> Turns {
        Turns(Stream::resume(carry))
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
            "attachment" => self.prompt(v, ts),
            "system" if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => {
                self.push(ts, TurnKind::Compact, 0);
            }
            _ => {}
        }
    }

    fn push(&mut self, ts: i64, kind: TurnKind, n: i64) {
        self.0.push(ts, kind, n);
    }

    fn user(&mut self, v: &Value, ts: i64) {
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
            return;
        };
        // A user line holds text blocks (or a string) and tool results; an interruption notice is
        // one of the texts.
        let mut interrupted = matches!(content, Value::String(t) if is_interrupt(t));
        if let Value::Array(blocks) = content {
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        interrupted |= b
                            .get("text")
                            .and_then(Value::as_str)
                            .is_some_and(is_interrupt);
                    }
                    Some("tool_result") => self.result(b, ts),
                    _ => {}
                }
            }
        }
        if interrupted {
            self.push(ts, TurnKind::Interrupt, 0);
        }
        self.prompt(v, ts);
    }

    /// The prompt of a line, if it holds one (`typed_prompt`): it ends the turn before it.
    fn prompt(&mut self, v: &Value, ts: i64) {
        if let Some(text) = typed_prompt(v) {
            self.0.close();
            self.push(ts, TurnKind::Prompt, text.chars().count() as i64);
        }
    }

    /// A failed tool result is a refusal when its text says the user or a rule said no.
    fn result(&mut self, b: &Value, ts: i64) {
        if b.get("is_error").and_then(Value::as_bool) != Some(true) {
            return;
        }
        let text = result_text(b.get("content"));
        let kind = if RE_REFUSAL.is_match(&text) {
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
        self.0.reply(ts, words);
    }

    /// The events, and the turn to carry to the next read.
    pub(super) fn finish(self) -> (Vec<TurnEvent>, Option<Value>) {
        self.0.finish()
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
        ) && !(line_type(line) == Some("attachment") && is_queued_prompt(line))
        {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(ty) = v.get("type").and_then(Value::as_str) {
            turns.line(&v, ty);
        }
    })?;
    Ok(turns.finish().0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turns::testing::{
        assert_every_split_matches_whole, end_times, kinds, read_in_two, wrapped,
    };

    const FIXTURE: &str = include_str!("../../../tests/fixtures/claude-turns.jsonl");
    /// A prompt that follows the fixture's last reply.
    const NEXT_PROMPT: &str = r#"{"type": "user", "timestamp": "2026-11-02T10:09:00.000Z", "message": {"content": "thanks, now ship it"}}"#;

    fn feed(turns: &mut Turns, text: &str) {
        for line in text.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            let ty = v["type"].as_str().unwrap().to_string();
            turns.line(&v, &ty);
        }
    }

    fn read(carry: Option<&Value>, text: &str) -> (Vec<TurnEvent>, Option<Value>) {
        let carry = carry.and_then(|c| wrapped(&Some(c.clone())));
        let mut t = Turns::resume(carry.as_ref());
        feed(&mut t, text);
        t.finish()
    }

    fn at(m: u32, s: u32) -> i64 {
        iso_to_ms(&format!("2026-11-02T10:{m:02}:{s:02}.000Z")).unwrap()
    }

    fn user_line(content: Value) -> Value {
        serde_json::json!({"type": "user", "timestamp": "2026-11-02T10:00:00.000Z",
            "message": {"content": content}})
    }

    #[test]
    fn a_whole_transcript_reports_every_kind_once_and_leaves_its_last_turn_open() {
        let (events, carry) = read(None, FIXTURE);
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
            ]
        );
        // the sub-agent's tool call, the command wrapper and the harness note are not turns
        let open = carry.unwrap();
        assert_eq!(
            (open["words"].as_i64(), open["ts"].as_i64()),
            (Some(8), Some(at(7, 0)))
        );
    }

    #[test]
    fn an_end_carries_the_time_of_the_last_reply_of_its_turn() {
        let (events, _) = read(None, &format!("{}\n{NEXT_PROMPT}", FIXTURE.trim_end()));
        assert_eq!(end_times(&events), vec![at(2, 10), at(7, 0)]);
    }

    #[test]
    fn every_split_point_gives_the_events_of_a_whole_read() {
        assert_every_split_matches_whole(FIXTURE, read);
        assert_every_split_matches_whole(&format!("{}\n{NEXT_PROMPT}", FIXTURE.trim_end()), read);
    }

    #[test]
    fn a_turn_read_in_two_goes_ends_once_at_its_last_reply() {
        // the first read stops after the first reply of the second turn, four minutes before its end
        let text = format!("{}\n{NEXT_PROMPT}", FIXTURE.trim_end());
        let (a, b) = read_in_two(&text, 13, read);
        assert_eq!(end_times(&a), vec![at(2, 10)]);
        assert_eq!(end_times(&b), vec![at(7, 0)]);
        assert_eq!(
            b.iter().find(|e| e.kind == TurnKind::End).map(|e| e.n),
            Some(8)
        );
    }

    #[test]
    fn a_turn_open_at_the_end_of_a_read_is_not_closed_by_the_next_read_that_goes_on_in_it() {
        let (a, b) = read_in_two(FIXTURE, 14, read);
        assert_eq!(end_times(&a), vec![at(2, 10)]);
        assert!(end_times(&b).is_empty());
    }

    #[test]
    fn a_prompt_with_an_image_is_a_prompt() {
        let mut t = Turns::resume(None);
        let v = user_line(serde_json::json!([
            {"type": "image", "source": {"type": "base64", "data": ""}},
            {"type": "text", "text": "still broken, see screenshot"}
        ]));
        t.line(&v, "user");
        assert_eq!(kinds(&t.finish().0), vec![(TurnKind::Prompt, 28)]);
    }

    #[test]
    fn the_feedback_on_a_rejected_tool_call_is_a_refusal_and_a_prompt() {
        use TurnKind::*;
        // as Claude Code writes it: the comment is the end of the tool result's own text
        let mut t = Turns::resume(None);
        let v = user_line(serde_json::json!([
            {"type": "tool_result", "is_error": true,
             "content": "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). To tell you how to proceed, the user said:\nuse rg instead"},
        ]));
        t.line(&v, "user");
        assert_eq!(kinds(&t.finish().0), vec![(Refusal, 0), (Prompt, 14)]);

        // a refusal with no comment, and the interruption notice that comes with it
        let mut t = Turns::resume(None);
        let v = user_line(serde_json::json!([
            {"type": "tool_result", "is_error": true,
             "content": "The user doesn't want to proceed with this tool use. The tool use was rejected."},
            {"type": "text", "text": "[Request interrupted by user for tool use]"}
        ]));
        t.line(&v, "user");
        assert_eq!(kinds(&t.finish().0), vec![(Refusal, 0), (Interrupt, 0)]);

        // a comment in a text block next to the result is a prompt too
        let mut t = Turns::resume(None);
        let v = user_line(serde_json::json!([
            {"type": "tool_result", "is_error": true, "content": "The user doesn't want to proceed."},
            {"type": "text", "text": "use rg instead"},
        ]));
        t.line(&v, "user");
        assert_eq!(kinds(&t.finish().0), vec![(Refusal, 0), (Prompt, 14)]);
    }

    #[test]
    fn a_prompt_that_starts_with_a_paste_or_a_slash_command_with_arguments_is_a_prompt() {
        use TurnKind::*;
        for (raw, n) in [
            (
                "<pasted_content id=\"a\">trace</pasted_content id=\"a\"> why does this fail?",
                "trace why does this fail?".len() as i64,
            ),
            (
                "<command-name>/review-pr</command-name><command-args>12 quickly</command-args>",
                "/review-pr 12 quickly".len() as i64,
            ),
        ] {
            let mut t = Turns::resume(None);
            t.line(&user_line(serde_json::json!(raw)), "user");
            assert_eq!(kinds(&t.finish().0), vec![(Prompt, n)], "{raw}");
        }
        // a bare slash command and a background-task notice are not prompts
        let mut t = Turns::resume(None);
        for raw in [
            "<command-name>/clear</command-name><command-args></command-args>",
            "<task-notification>done</task-notification>",
        ] {
            t.line(&user_line(serde_json::json!(raw)), "user");
        }
        assert!(t.finish().0.is_empty());
    }

    #[test]
    fn a_prompt_typed_while_the_agent_worked_is_a_prompt_that_closes_the_turn() {
        use TurnKind::*;
        let queued = |origin: Value, mode: &str, meta: bool| {
            serde_json::json!({"type": "attachment", "timestamp": "2026-11-02T10:09:00.000Z",
                "attachment": {"type": "queued_command", "prompt": "also add a test",
                    "commandMode": mode, "origin": origin, "isMeta": meta}})
        };
        let human = serde_json::json!({"kind": "human"});
        let mut t = Turns::resume(None);
        let reply = serde_json::json!({"type": "assistant", "timestamp": "2026-11-02T10:08:00.000Z",
            "message": {"content": [{"type": "text", "text": "working on it"}]}});
        t.line(&reply, "assistant");
        t.line(&queued(human.clone(), "prompt", false), "attachment");
        // a notice of a background task, another agent's message and a reminder are not prompts
        t.line(
            &queued(human.clone(), "task-notification", false),
            "attachment",
        );
        t.line(
            &queued(serde_json::json!({"kind": "peer"}), "prompt", false),
            "attachment",
        );
        t.line(&queued(human, "prompt", true), "attachment");
        assert_eq!(kinds(&t.finish().0), vec![(End, 3), (Prompt, 15)]);
    }

    #[test]
    fn a_list_with_only_tool_results_or_harness_text_is_no_prompt() {
        let mut t = Turns::resume(None);
        t.line(
            &user_line(serde_json::json!([{"type": "tool_result", "content": "fine"}])),
            "user",
        );
        t.line(
            &user_line(serde_json::json!([{"type": "text", "text": "<system-reminder>x</system-reminder>"}])),
            "user",
        );
        assert!(t.finish().0.is_empty());
    }

    #[test]
    fn what_makes_a_failed_result_a_refusal() {
        let mut t = Turns::resume(None);
        for (text, want) in [
            ("Permission to use Bash has been denied.", TurnKind::Refusal),
            (
                "Permission for this action has been denied.",
                TurnKind::Refusal,
            ),
            (
                "The user doesn't want to take this action right now. STOP what you are doing.",
                TurnKind::Refusal,
            ),
            ("Exit code 1", TurnKind::ToolError),
            // a command that failed with words a refusal has is still a failed command
            (
                "Exit code 1\nbash: ./run.sh: Permission denied",
                TurnKind::ToolError,
            ),
            (
                "Exit code 1\n! [remote rejected] main -> main (permission denied)",
                TurnKind::ToolError,
            ),
            ("Error: file not allowed here", TurnKind::ToolError),
        ] {
            let v = user_line(
                serde_json::json!([{"type": "tool_result", "is_error": true, "content": text}]),
            );
            t.line(&v, "user");
            assert_eq!(t.0.last().map(|e| e.kind), Some(want), "{text}");
        }
        for text in crate::turns::testing::NOT_REFUSALS {
            let v = user_line(
                serde_json::json!([{"type": "tool_result", "is_error": true, "content": text}]),
            );
            t.line(&v, "user");
            assert_eq!(
                t.0.last().map(|e| e.kind),
                Some(TurnKind::ToolError),
                "{text}"
            );
        }
    }

    #[test]
    fn a_result_that_did_not_fail_and_an_unsigned_thinking_block_are_nothing() {
        let mut t = Turns::resume(None);
        let ok = user_line(serde_json::json!([{"type": "tool_result", "content": "fine"}]));
        t.line(&ok, "user");
        let think = serde_json::json!({"type":"assistant","timestamp":"2026-11-02T10:00:01.000Z",
            "message":{"content":[{"type":"thinking","thinking":"hm","signature":""}]}});
        t.line(&think, "assistant");
        assert!(t.0.last().is_none());
    }

    #[test]
    fn an_empty_transcript_has_no_turns_and_nothing_to_carry() {
        let (events, carry) = Turns::resume(None).finish();
        assert!(events.is_empty() && carry.is_none());
    }

    #[test]
    fn the_whole_file_reader_matches_the_streaming_one() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        std::fs::write(&p, FIXTURE).unwrap();
        assert_eq!(read_events(&p).unwrap(), read(None, FIXTURE).0);
    }
}
