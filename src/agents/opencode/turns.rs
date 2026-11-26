//! The turn events of an OpenCode session, for the steering clock (`crate::turns`), from the same
//! messages and parts `events` reads, in either layout.
//!
//! A turn ends with the completed time of the last assistant message before the next prompt (see
//! `crate::turns::stream`). OpenCode records the thinking of a message as `tokens.reasoning`, so a
//! thought is one message's reasoning tokens. A message that was stopped by the user carries a
//! `MessageAbortedError`; a tool part that failed has `state.status` `error`, and is a refusal
//! when its error says the user or a rule rejected the call.

use super::events::{n, s, text_parts};
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

static RE_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)rejected|prevents you|denied|not allowed|permission").expect("static regex")
});

/// Turns the messages of a session, in order, into turn events. The session is read whole, so the
/// last turn is closed at its end.
#[derive(Default)]
struct Builder(Stream);

impl Builder {
    fn prompt(&mut self, ts: i64, text: &str) {
        self.0.close();
        self.0
            .push(ts, TurnKind::Prompt, text.chars().count() as i64);
    }

    /// An assistant message: `m` its data, `content` its parts, `ts` when it began.
    fn assistant(&mut self, ts: i64, m: &Value, content: &[Value]) {
        let done = m.get("time").map_or(0, |t| n(t, "completed")).max(ts);
        for p in content {
            match s(p, "type") {
                "tool" => self.tool(ts, p),
                _ => {}
            }
        }
        let reasoning = m.get("tokens").map_or(0, |t| n(t, "reasoning"));
        if reasoning > 0 {
            self.0.push(ts, TurnKind::Think, reasoning);
        }
        let words = text_parts(content).split_whitespace().count() as i64;
        self.0.reply(done, words);
        let aborted = m
            .get("error")
            .is_some_and(|e| s(e, "name") == "MessageAbortedError");
        if aborted {
            self.0.push(done, TurnKind::Interrupt, 0);
        }
    }

    fn tool(&mut self, ts: i64, p: &Value) {
        let state = p.get("state").unwrap_or(&Value::Null);
        let at = state.get("time").map_or(0, |t| n(t, "start")).max(ts);
        self.0.push(at, TurnKind::Tool, 0);
        if s(state, "status") == "error" {
            let kind = if RE_REFUSAL.is_match(s(state, "error")) {
                TurnKind::Refusal
            } else {
                TurnKind::ToolError
            };
            self.0.push(at, kind, 0);
        }
    }

    fn compaction(&mut self, ts: i64) {
        self.0.push(ts, TurnKind::Compact, 0);
    }

    fn finish(self) -> Vec<TurnEvent> {
        self.0.finish(true).0
    }
}

/// Turn events of the older layout: `messages` are (id, time created in ms, `data`) in order,
/// `parts` the `data` of each message's parts in order.
pub fn from_v1(
    messages: &[(String, i64, Value)],
    parts: &HashMap<String, Vec<Value>>,
) -> Vec<TurnEvent> {
    let mut b = Builder::default();
    for (id, created, m) in messages {
        let ts = m.get("time").map_or(*created, |t| n(t, "created")).max(0);
        let ts = if ts == 0 { *created } else { ts };
        let ps = parts.get(id).map(Vec::as_slice).unwrap_or(&[]);
        match s(m, "role") {
            "user" => {
                if ps.iter().any(|p| s(p, "type") == "compaction") {
                    b.compaction(ts);
                }
                let text = text_parts(ps);
                if !text.is_empty() {
                    b.prompt(ts, &text);
                }
            }
            "assistant" => b.assistant(ts, m, ps),
            _ => {}
        }
    }
    b.finish()
}

/// Turn events of `session_v2`: `messages` are (id, type, time created, `data`) in `seq` order and
/// `parts` the `data` of each assistant message's `session_part` rows in `ordinal` order.
pub fn from_v2(
    messages: &[(String, String, i64, Value)],
    parts: &HashMap<String, Vec<Value>>,
) -> Vec<TurnEvent> {
    let mut b = Builder::default();
    for (id, kind, ts, m) in messages {
        match kind.as_str() {
            "user" => {
                let text = s(m, "text").trim();
                if !text.is_empty() {
                    b.prompt(*ts, text);
                }
            }
            "assistant" => {
                let inline = m.get("content").and_then(Value::as_array);
                let content = match inline {
                    Some(c) if !c.is_empty() => c.clone(),
                    _ => parts.get(id).cloned().unwrap_or_default(),
                };
                b.assistant(*ts, m, &content);
            }
            "compaction" => b.compaction(*ts),
            _ => {}
        }
    }
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/opencode-turns.json");

    /// The fixture's older-layout messages and parts.
    fn v1() -> (Vec<(String, i64, Value)>, HashMap<String, Vec<Value>>) {
        let f: Value = serde_json::from_str(FIXTURE).unwrap();
        let msgs = f["v1"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m[0].as_str().unwrap().to_string(),
                    m[1].as_i64().unwrap(),
                    m[2].clone(),
                )
            })
            .collect();
        let parts = f["v1"]["parts"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_array().unwrap().clone()))
            .collect();
        (msgs, parts)
    }

    fn kinds(events: &[TurnEvent]) -> Vec<(TurnKind, i64)> {
        events.iter().map(|e| (e.kind, e.n)).collect()
    }

    #[test]
    fn a_session_reports_every_kind_once() {
        let (msgs, parts) = v1();
        let events = from_v1(&msgs, &parts);
        use TurnKind::*;
        assert_eq!(
            kinds(&events),
            vec![
                (Prompt, 20),
                (Tool, 0),
                (Tool, 0),
                (ToolError, 0),
                (Tool, 0),
                (Refusal, 0),
                (Think, 300),
                (Interrupt, 0),
                (Compact, 0),
                (End, 4),
                (Prompt, 9),
                (End, 2),
            ]
        );
    }

    #[test]
    fn an_end_is_the_completed_time_of_the_last_assistant_message_of_its_turn() {
        let (msgs, parts) = v1();
        let events = from_v1(&msgs, &parts);
        let ends: Vec<i64> = events
            .iter()
            .filter(|e| e.kind == TurnKind::End)
            .map(|e| e.ts)
            .collect();
        let at = |m: i64, s: i64| (1_793_696_400 + m * 60 + s) * 1000;
        assert_eq!(ends, vec![at(0, 40), at(6, 0)]);
    }

    #[test]
    fn reading_the_session_again_gives_the_same_events() {
        // OpenCode sessions are read whole each time, so a re-read mid-turn replaces the events
        // and the open turn is closed once
        let (msgs, parts) = v1();
        let whole = from_v1(&msgs, &parts);
        let first_half = from_v1(&msgs[..2], &parts);
        let ends = |e: &[TurnEvent]| e.iter().filter(|e| e.kind == TurnKind::End).count();
        assert_eq!(ends(&first_half), 1);
        assert_eq!(ends(&whole), 2);
        assert_eq!(from_v1(&msgs, &parts), whole);
    }

    #[test]
    fn the_newer_layout_gives_the_same_kinds_from_inline_or_stored_parts() {
        let msgs = vec![
            (
                "a".to_string(),
                "user".to_string(),
                1000,
                json!({"text": "hi there"}),
            ),
            (
                "b".to_string(),
                "assistant".to_string(),
                2000,
                json!({"time": {"completed": 3000}, "tokens": {"reasoning": 7},
                    "content": [{"type": "tool", "state": {"status": "error", "error": "denied by rule"}},
                                {"type": "text", "text": "ok done"}]}),
            ),
            ("c".to_string(), "compaction".to_string(), 4000, json!({})),
            (
                "d".to_string(),
                "assistant".to_string(),
                5000,
                json!({"time": {"completed": 6000}}),
            ),
        ];
        let mut parts = HashMap::new();
        parts.insert(
            "d".to_string(),
            vec![json!({"type": "text", "text": "late reply"})],
        );
        let events = from_v2(&msgs, &parts);
        use TurnKind::*;
        assert_eq!(
            kinds(&events),
            vec![
                (Prompt, 8),
                (Tool, 0),
                (Refusal, 0),
                (Think, 7),
                (Compact, 0),
                (End, 4)
            ]
        );
        assert_eq!(events.last().map(|e| e.ts), Some(6000));
    }

    #[test]
    fn nothing_that_did_not_fail_is_an_error_and_an_empty_session_has_no_turns() {
        let parts = HashMap::new();
        assert!(from_v1(&[], &parts).is_empty());
        let msgs = vec![(
            "m".to_string(),
            1,
            json!({"role": "assistant", "tokens": {"reasoning": 0}}),
        )];
        let mut parts = HashMap::new();
        parts.insert(
            "m".to_string(),
            vec![json!({"type": "tool", "state": {"status": "completed"}})],
        );
        let events = from_v1(&msgs, &parts);
        assert_eq!(
            kinds(&events),
            vec![(TurnKind::Tool, 0), (TurnKind::End, 0)]
        );
    }
}
