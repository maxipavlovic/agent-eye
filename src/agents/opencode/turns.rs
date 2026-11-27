//! The turn events of an OpenCode session, for the steering clock (`crate::turns`), from the same
//! messages and parts `events` reads, in either layout.
//!
//! A turn ends with the completed time of the last assistant message before the next prompt (see
//! `crate::turns::stream`). OpenCode records the thinking of a message as `tokens.reasoning`, so a
//! thought is one message's reasoning tokens. A message that was stopped by the user carries a
//! `MessageAbortedError`; a tool part that failed has `state.status` `error`, and is a refusal
//! when its error is OpenCode's own message for a permission decision (the user said no, or a
//! rule did), which is a message and not a failure of the tool. The wording is from OpenCode's
//! source and not yet seen in a real session.

use super::events::{n, s, text_parts, Ev};
use crate::turns::stream::Stream;
use crate::turns::{TurnEvent, TurnKind};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

static RE_REFUSAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(The user rejected permission|The user has specified a rule which prevents you)",
    )
    .expect("static regex")
});

/// Turns the messages of a session, in order, into turn events. The last turn has no `End` until
/// a prompt follows it, as for every agent.
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
        for p in content.iter().filter(|p| s(p, "type") == "tool") {
            self.tool(ts, p);
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
        self.0.finish().0
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

/// Turn events of the session an archive copy from before eye kept them holds, from its native
/// export when that has the older layout's messages (`info` and `parts` each). None when it has not.
pub fn from_native(native: &Value) -> Option<Vec<TurnEvent>> {
    let mut messages = vec![];
    let mut parts = HashMap::new();
    for m in native.get("messages")?.as_array()? {
        let info = m.get("info")?;
        let id = s(info, "id");
        if id.is_empty() || s(info, "role").is_empty() {
            return None;
        }
        let created = info.get("time").map_or(0, |t| n(t, "created"));
        messages.push((id.to_string(), created, info.clone()));
        let ps = m.get("parts").and_then(Value::as_array).cloned();
        parts.insert(id.to_string(), ps.unwrap_or_default());
    }
    Some(from_v1(&messages, &parts))
}

/// Turn events from what an archive copy keeps of every session: prompts, tool calls and the time
/// of the last reply of each turn. That is all its events say, so a copy that has no native export
/// has no thinking, errors, refusals, interrupts or compactions.
pub fn from_events(events: &[Ev]) -> Vec<TurnEvent> {
    let mut b = Builder::default();
    for e in events {
        match e {
            Ev::Prompt { ts, text } => b.prompt(*ts, text),
            Ev::Reply { ts, text } => b.0.reply(*ts, text.split_whitespace().count() as i64),
            Ev::Tool { ts, .. } => {
                b.0.reply(*ts, 0);
                b.0.push(*ts, TurnKind::Tool, 0);
            }
            Ev::Usage { .. } => {}
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
    use crate::turns::testing::{end_times, kinds};
    use serde_json::json;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/opencode-turns.json");

    type Messages = Vec<(String, i64, Value)>;
    type Parts = HashMap<String, Vec<Value>>;

    /// The fixture's older-layout messages and parts.
    fn v1() -> (Messages, Parts) {
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

    #[test]
    fn only_a_permission_decision_is_a_refusal_not_an_error_that_mentions_permission() {
        let kind = |error: &str| {
            let mut b = Builder::default();
            b.tool(
                1000,
                &json!({"type": "tool", "state": {"status": "error", "error": error}}),
            );
            b.finish().last().map(|e| e.kind)
        };
        for said in [
            "The user rejected permission to use this specific tool call.",
            "The user rejected permission to use this specific tool call. You may try again with the following guidance: use rg",
            "The user has specified a rule which prevents you from using this specific tool call.",
        ] {
            assert_eq!(kind(said), Some(TurnKind::Refusal), "{said}");
        }
        for failed in [
            "EACCES: permission denied, open '/etc/hosts'",
            "git push was rejected: non-fast-forward",
            "command not allowed in this shell",
        ] {
            assert_eq!(kind(failed), Some(TurnKind::ToolError), "{failed}");
        }
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
            ]
        );
    }

    #[test]
    fn an_end_is_the_completed_time_of_the_last_assistant_message_of_its_turn() {
        let (mut msgs, parts) = v1();
        let at = |m: i64, s: i64| (1_793_696_400 + m * 60 + s) * 1000;
        assert_eq!(end_times(&from_v1(&msgs, &parts)), vec![at(0, 40)]);
        // the second turn ends when a prompt follows it
        msgs.push((
            "next".into(),
            at(9, 0),
            json!({"role": "user", "time": {"created": at(9, 0)}}),
        ));
        let mut parts = parts;
        parts.insert(
            "next".into(),
            vec![json!({"type": "text", "text": "ship it"})],
        );
        assert_eq!(
            end_times(&from_v1(&msgs, &parts)),
            vec![at(0, 40), at(6, 0)]
        );
    }

    #[test]
    fn reading_the_session_again_gives_the_same_events() {
        // OpenCode sessions are read whole each time, so a re-read mid-turn replaces the events and
        // the turn that was open gets its one `End` when the prompt after it is read
        let (msgs, parts) = v1();
        let whole = from_v1(&msgs, &parts);
        let first_half = from_v1(&msgs[..2], &parts);
        assert!(end_times(&first_half).is_empty());
        assert_eq!(end_times(&whole).len(), 1);
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
                    "content": [{"type": "tool", "state": {"status": "error", "error": "The user has specified a rule which prevents you from using this specific tool call."}},
                                {"type": "text", "text": "ok done"}]}),
            ),
            ("c".to_string(), "compaction".to_string(), 4000, json!({})),
            (
                "d".to_string(),
                "assistant".to_string(),
                5000,
                json!({"time": {"completed": 6000}}),
            ),
            (
                "e".to_string(),
                "user".to_string(),
                7000,
                json!({"text": "next"}),
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
                (End, 4),
                (Prompt, 4)
            ]
        );
        assert_eq!(end_times(&events), vec![6000]);
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
        assert_eq!(kinds(&events), vec![(TurnKind::Tool, 0)]);
    }

    /// The fixture's messages in the shape of `opencode session export`.
    fn native() -> Value {
        let (msgs, parts) = v1();
        let messages: Vec<Value> = msgs
            .iter()
            .map(|(id, _, m)| {
                let mut info = m.clone();
                info["id"] = id.as_str().into();
                json!({"info": info, "parts": parts.get(id).cloned().unwrap_or_default()})
            })
            .collect();
        json!({"info": {"id": "ses_1"}, "messages": messages})
    }

    #[test]
    fn an_archive_copy_without_turns_gets_them_from_its_native_export() {
        let (msgs, parts) = v1();
        assert_eq!(from_native(&native()), Some(from_v1(&msgs, &parts)));
        // a layout this reads no messages of is left to the events
        let v2 = json!({"messages": [{"info": {"id": "a", "type": "user"}, "parts": []}]});
        assert_eq!(from_native(&v2), None);
        assert_eq!(from_native(&json!({"info": {}})), None);
    }

    #[test]
    fn the_events_of_a_copy_give_prompts_tools_and_the_end_of_each_turn() {
        let events = vec![
            Ev::Prompt {
                ts: 1000,
                text: "fix it".into(),
            },
            Ev::Tool {
                ts: 2000,
                line: "bash".into(),
            },
            Ev::Reply {
                ts: 3000,
                text: "done now".into(),
            },
            Ev::Prompt {
                ts: 9000,
                text: "thanks".into(),
            },
        ];
        use TurnKind::*;
        let turns = from_events(&events);
        assert_eq!(
            kinds(&turns),
            vec![(Prompt, 6), (Tool, 0), (End, 2), (Prompt, 6)]
        );
        assert_eq!(end_times(&turns), vec![3000]);
        assert!(from_events(&[]).is_empty());
    }

    #[test]
    fn a_session_archived_before_turns_were_kept_is_read_with_them() {
        let dir = tempfile::tempdir().unwrap();
        let old = |native: Option<Value>| {
            let l = crate::agents::opencode::events::Loaded {
                eye_export: 1,
                id: "ses_1".into(),
                events: vec![Ev::Prompt {
                    ts: 1000,
                    text: "hello".into(),
                }],
                native,
                ..Default::default()
            };
            let p = dir.path().join(if l.native.is_some() {
                "n.json"
            } else {
                "e.json"
            });
            std::fs::write(&p, serde_json::to_vec(&l).unwrap()).unwrap();
            crate::agents::opencode::store::load(&p).unwrap().turns
        };
        // from the events: one prompt
        assert_eq!(kinds(&old(None)), vec![(TurnKind::Prompt, 5)]);
        // from the native export: all the kinds it has
        let from_native = old(Some(native()));
        assert!(from_native.iter().any(|e| e.kind == TurnKind::Refusal));
    }
}
