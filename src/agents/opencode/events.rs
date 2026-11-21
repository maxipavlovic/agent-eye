//! A session of OpenCode as a list of events, whichever way it was stored. The message and part
//! shapes follow `packages/schema/src/v1/session.ts` (the older tables and JSON files: messages with
//! `role`, parts with `type` `text` / `tool` / `step-finish`) and `packages/schema/src/session-message.ts`
//! (`session_v2`: messages with `type` `user` / `assistant`, an assistant's `content` in
//! `session_part` rows). See `notes` in `docs/agents.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// One thing that happened in a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Ev {
    Prompt {
        ts: i64,
        text: String,
    },
    Reply {
        ts: i64,
        text: String,
    },
    Tool {
        ts: i64,
        line: String,
    },
    Usage {
        ts: i64,
        model: String,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_write: i64,
    },
}

impl Ev {
    pub fn ts(&self) -> i64 {
        match self {
            Ev::Prompt { ts, .. }
            | Ev::Reply { ts, .. }
            | Ev::Tool { ts, .. }
            | Ev::Usage { ts, .. } => *ts,
        }
    }
}

/// A session read whole. This is also what the archive keeps (`eye_export` marks it).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Loaded {
    pub eye_export: u8,
    pub id: String,
    pub directory: String,
    pub title: Option<String>,
    pub events: Vec<Ev>,
    /// What `opencode session export` printed for the session, when OpenCode could be asked: what
    /// `opencode session import` needs to put the session back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<Value>,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn n(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_f64).map_or(0, |x| x as i64)
}

fn one_line(t: &str, max: usize) -> String {
    t.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// A tool call as one line: `$ <command>` for a shell, else the name and its first argument.
pub fn tool_line(name: &str, input: &Value) -> String {
    let arg = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
            .map(|a| one_line(a, 120))
    };
    if matches!(name, "bash" | "shell") {
        if let Some(c) = arg(&["command"]) {
            return format!("$ {c}");
        }
    }
    match arg(&["filePath", "path", "pattern", "url", "query", "description"]) {
        Some(a) => format!("{} {a}", one_line(name, 40)),
        None => one_line(name, 120),
    }
}

fn usage(ts: i64, model: &str, t: &Value) -> Option<Ev> {
    let cache = t.get("cache").unwrap_or(&Value::Null);
    let e = Ev::Usage {
        ts,
        model: model.to_string(),
        input: n(t, "input"),
        output: n(t, "output") + n(t, "reasoning"),
        cache_read: n(cache, "read"),
        cache_write: n(cache, "write"),
    };
    match &e {
        Ev::Usage {
            input,
            output,
            cache_read,
            cache_write,
            ..
        } if input + output + cache_read + cache_write > 0 => Some(e),
        _ => None,
    }
}

fn text_parts(parts: &[Value]) -> String {
    parts
        .iter()
        .filter(|p| s(p, "type") == "text")
        .filter(|p| !p.get("synthetic").and_then(Value::as_bool).unwrap_or(false))
        .filter(|p| !p.get("ignored").and_then(Value::as_bool).unwrap_or(false))
        .map(|p| s(p, "text").trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Events of the older shape: `messages` are (id, time created in ms, `data`) in order, `parts` are
/// the `data` of each message's parts in order.
pub fn from_v1(messages: &[(String, i64, Value)], parts: &HashMap<String, Vec<Value>>) -> Vec<Ev> {
    let mut out = vec![];
    for (id, created, m) in messages {
        let ts = m.get("time").map_or(*created, |t| n(t, "created")).max(0);
        let ts = if ts == 0 { *created } else { ts };
        let ps = parts.get(id).map(Vec::as_slice).unwrap_or(&[]);
        match s(m, "role") {
            "user" => {
                let text = text_parts(ps);
                if !text.is_empty() {
                    out.push(Ev::Prompt { ts, text });
                }
            }
            "assistant" => {
                for p in ps.iter().filter(|p| s(p, "type") == "tool") {
                    let input = p
                        .get("state")
                        .and_then(|st| st.get("input"))
                        .unwrap_or(&Value::Null);
                    out.push(Ev::Tool {
                        ts,
                        line: tool_line(s(p, "tool"), input),
                    });
                }
                let text = text_parts(ps);
                if !text.is_empty() {
                    out.push(Ev::Reply { ts, text });
                }
                if let Some(u) = m.get("tokens").and_then(|t| usage(ts, s(m, "modelID"), t)) {
                    out.push(u);
                }
            }
            _ => {}
        }
    }
    out
}

/// Events of `session_v2`: `messages` are (id, type, time created, `data`) in `seq` order and `parts`
/// the `data` of each assistant message's `session_part` rows in `ordinal` order.
pub fn from_v2(
    messages: &[(String, String, i64, Value)],
    parts: &HashMap<String, Vec<Value>>,
) -> Vec<Ev> {
    let mut out = vec![];
    for (id, kind, ts, m) in messages {
        match kind.as_str() {
            "user" => {
                let text = s(m, "text").trim();
                if !text.is_empty() {
                    out.push(Ev::Prompt {
                        ts: *ts,
                        text: text.to_string(),
                    });
                }
            }
            "assistant" => {
                let inline: Vec<Value> = m
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let content = if inline.is_empty() {
                    parts.get(id).cloned().unwrap_or_default()
                } else {
                    inline
                };
                for p in content.iter().filter(|p| s(p, "type") == "tool") {
                    let input = p
                        .get("state")
                        .and_then(|st| st.get("input"))
                        .unwrap_or(&Value::Null);
                    out.push(Ev::Tool {
                        ts: *ts,
                        line: tool_line(s(p, "name"), input),
                    });
                }
                let text = text_parts(&content);
                if !text.is_empty() {
                    out.push(Ev::Reply { ts: *ts, text });
                }
                let model = m.get("model").map_or("", |x| s(x, "id"));
                if let Some(u) = m.get("tokens").and_then(|t| usage(*ts, model, t)) {
                    out.push(u);
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_messages_become_events() {
        let msgs = vec![
            (
                "m1".to_string(),
                10,
                json!({"role": "user", "time": {"created": 1000}}),
            ),
            (
                "m2".to_string(),
                20,
                json!({"role": "assistant", "modelID": "big-1", "time": {"created": 2000},
                "tokens": {"input": 7, "output": 3, "reasoning": 2, "cache": {"read": 5, "write": 1}}}),
            ),
        ];
        let mut parts = HashMap::new();
        parts.insert(
            "m1".to_string(),
            vec![
                json!({"type": "text", "text": "fix it"}),
                json!({"type": "text", "text": "ctx", "synthetic": true}),
            ],
        );
        parts.insert("m2".to_string(), vec![
            json!({"type": "tool", "tool": "bash", "callID": "c", "state": {"status": "completed", "input": {"command": "ls  -la"}}}),
            json!({"type": "text", "text": "done"}),
            json!({"type": "step-finish"}),
        ]);
        let ev = from_v1(&msgs, &parts);
        assert_eq!(
            ev[0],
            Ev::Prompt {
                ts: 1000,
                text: "fix it".into()
            }
        );
        assert_eq!(
            ev[1],
            Ev::Tool {
                ts: 2000,
                line: "$ ls -la".into()
            }
        );
        assert_eq!(
            ev[2],
            Ev::Reply {
                ts: 2000,
                text: "done".into()
            }
        );
        assert_eq!(
            ev[3],
            Ev::Usage {
                ts: 2000,
                model: "big-1".into(),
                input: 7,
                output: 5,
                cache_read: 5,
                cache_write: 1
            }
        );
    }

    #[test]
    fn v2_takes_the_content_inline_or_from_parts() {
        let msgs = vec![
            (
                "msg_1".to_string(),
                "user".to_string(),
                5,
                json!({"text": "hi"}),
            ),
            (
                "msg_2".to_string(),
                "assistant".to_string(),
                6,
                json!({"model": {"id": "m"}, "tokens": {"input": 1, "output": 1, "reasoning": 0, "cache": {"read": 0, "write": 0}}}),
            ),
            (
                "msg_3".to_string(),
                "synthetic".to_string(),
                7,
                json!({"text": "x"}),
            ),
        ];
        let mut parts = HashMap::new();
        parts.insert(
            "msg_2".to_string(),
            vec![
                json!({"type": "text", "text": "yo"}),
                json!({"type": "tool", "name": "read", "state": {"input": {"filePath": "/a/b"}}}),
            ],
        );
        let ev = from_v2(&msgs, &parts);
        assert_eq!(ev.len(), 4);
        assert_eq!(
            ev[1],
            Ev::Tool {
                ts: 6,
                line: "read /a/b".into()
            }
        );
        assert_eq!(
            ev[2],
            Ev::Reply {
                ts: 6,
                text: "yo".into()
            }
        );
    }
}
