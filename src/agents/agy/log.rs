//! The JSON log of a conversation: one line per step, written by agy next to the conversation
//! (`transcript_full.jsonl`). A step has `step_index`, `source`, `type` (`USER_INPUT`,
//! `PLANNER_RESPONSE`, `GENERIC` for a tool result, `SYSTEM_MESSAGE`, `ERROR_MESSAGE`), `status`,
//! `created_at` (UTC, seconds), and by type `content`, `thinking`, `tool_calls` and the tokens of
//! the model call (`input_tokens` without the cache, `cache_read_tokens`, `output_tokens` with the
//! thinking). The prompt is wrapped in `<USER_REQUEST>`.

use crate::agents::append_only::for_each_line;
use crate::agents::outputs::{cap_str, scan_jira, scan_jira_urls, scan_pr, Seen, RESULT_SCAN_CAP};
use crate::agents::{Delta, TailItem, UsageMsg, PROMPTS_CAP, REPLIES_CAP};
use crate::conversation::Message;
use crate::model::Tokens;
use crate::util::{head_chars, iso_to_ms, tail_chars, trim_front_bytes};
use crate::worked::WorkEvent;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

const MSG_CAP: usize = 4_000;
const TAIL_TEXTS: usize = 30;
const TAIL_ITEMS_MAX: usize = 90;

/// One step of the log.
#[derive(Debug, Default, Clone)]
pub struct Step {
    pub kind: String,
    pub ts: Option<i64>,
    pub content: String,
    pub error: Option<String>,
    /// (name, arguments)
    pub tools: Vec<(String, Value)>,
    pub tokens: Option<Tokens>,
}

pub fn decode(line: &[u8]) -> Option<Step> {
    let v: Value = serde_json::from_slice(line).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let n = |k: &str| v.get(k).and_then(Value::as_i64);
    let tokens = n("input_tokens")
        .or(n("output_tokens"))
        .or(n("cache_read_tokens"))
        .map(|_| Tokens {
            input: n("input_tokens").unwrap_or(0),
            output: n("output_tokens").unwrap_or(0),
            cache_write: 0,
            cache_read: n("cache_read_tokens").unwrap_or(0),
            subagents_total: 0,
        });
    let tools = v
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| {
                    (
                        c.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        c.get("args").cloned().unwrap_or(Value::Null),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Step {
        kind: s("type"),
        ts: v
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(iso_to_ms),
        content: s("content"),
        error: v.get("error").and_then(Value::as_str).map(String::from),
        tools,
        tokens,
    })
}

/// What the user typed: the text inside `<USER_REQUEST>` (the rest of the step is metadata).
pub fn prompt_text(content: &str) -> Option<String> {
    let t = match content.split_once("<USER_REQUEST>") {
        Some((_, rest)) => rest.split("</USER_REQUEST>").next().unwrap_or(rest),
        None => content,
    };
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn one_line(s: &str, max: usize) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// A tool call as one line: `$ <command>` for a command, else the name and its first argument.
pub fn tool_line(name: &str, args: &Value) -> String {
    let arg = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .map(|s| one_line(s, 120))
    };
    if name == "run_command" {
        if let Some(c) = arg(&["CommandLine"]) {
            return format!("$ {c}");
        }
    }
    match arg(&[
        "AbsolutePath",
        "TargetFile",
        "Path",
        "query",
        "Query",
        "Url",
        "url",
    ]) {
        Some(a) => format!("{} {a}", one_line(name, 40)),
        None => one_line(name, 120),
    }
}

fn each_step(path: &Path, from: u64, mut each: impl FnMut(Step)) -> Result<u64> {
    for_each_line(path, from, |l| {
        if let Some(s) = decode(l) {
            each(s);
        }
    })
}

/// The result of reading the log: the delta and the number of model calls seen (so far, with the
/// count before this read).
pub struct Parsed {
    pub delta: Delta,
    pub model_calls: usize,
}

/// Read the log from byte `from`. `calls_before` is the number of model calls read earlier
/// (`models[i]` is the model of the i-th call).
pub fn parse(
    path: &Path,
    from: u64,
    calls_before: usize,
    models: &dyn Fn() -> Vec<String>,
) -> Result<Parsed> {
    let mut d = Delta::default();
    let mut seen = Seen::new();
    let mut calls = calls_before;
    let mut models_cache: Option<Vec<String>> = None;
    let end = each_step(path, from, |s| {
        d.lines += 1;
        if let Some(ts) = s.ts {
            d.first_ts = Some(d.first_ts.map_or(ts, |t| t.min(ts)));
            d.last_ts = Some(d.last_ts.map_or(ts, |t| t.max(ts)));
        }
        let ts = s.ts.unwrap_or(0);
        match s.kind.as_str() {
            "USER_INPUT" => {
                if let Some(p) = prompt_text(&s.content) {
                    d.prompt_count += 1;
                    for pre in scan_jira(&mut d, &mut seen, &p, ts) {
                        d.jira_prompt_prefixes.insert(pre);
                    }
                    let p = head_chars(&p, MSG_CAP);
                    d.first_prompt.get_or_insert_with(|| head_chars(&p, 2_000));
                    d.last_user_text = Some(head_chars(&p, 1_500));
                    d.prompts_text.push_str(&p);
                    d.prompts_text.push('\n');
                    if d.prompts_text.len() > PROMPTS_CAP * 2 {
                        trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
                    }
                    if let Some(t) = s.ts {
                        d.events.push(WorkEvent {
                            ts: t,
                            prompt: true,
                        });
                    }
                }
            }
            "PLANNER_RESPONSE" => {
                if let Some(t) = s.ts {
                    d.events.push(WorkEvent {
                        ts: t,
                        prompt: false,
                    });
                }
                let text = s.content.trim();
                if !text.is_empty() {
                    d.reply_count += 1;
                    scan_jira(&mut d, &mut seen, text, ts);
                    scan_pr(&mut d, &mut seen, text, ts);
                    scan_jira_urls(&mut d, text);
                    d.last_assistant_text = Some(tail_chars(text, 1_500));
                    d.replies_text.push_str(&head_chars(text, MSG_CAP));
                    d.replies_text.push('\n');
                    if d.replies_text.len() > REPLIES_CAP * 2 {
                        trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
                    }
                }
                if let Some(t) = s.tokens {
                    let models = models_cache.get_or_insert_with(models);
                    let model = models
                        .get(calls)
                        .or(models.last())
                        .cloned()
                        .unwrap_or_default();
                    d.usage.input += t.input;
                    d.usage.output += t.output;
                    d.usage.cache_read += t.cache_read;
                    d.usage_msgs.push(UsageMsg {
                        ts,
                        model,
                        input: t.input,
                        output: t.output,
                        cache_write: 0,
                        cache_read: t.cache_read,
                    });
                    calls += 1;
                }
            }
            "GENERIC" => {
                if let Some(t) = s.ts {
                    d.events.push(WorkEvent {
                        ts: t,
                        prompt: false,
                    });
                }
                let text = cap_str(&s.content, RESULT_SCAN_CAP);
                scan_pr(&mut d, &mut seen, text, ts);
                scan_jira_urls(&mut d, text);
            }
            _ => {}
        }
    })?;
    d.marker = end.to_string();
    d.bytes_read = end.saturating_sub(from);
    trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
    trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
    Ok(Parsed {
        delta: d,
        model_calls: calls,
    })
}

/// Prompts and replies of the log (a turn's replies joined, with its tool-call count).
pub fn conversation(path: &Path) -> Result<Vec<Message>> {
    let mut out: Vec<Message> = vec![];
    let (mut texts, mut tools, mut ts): (Vec<String>, u32, Option<i64>) = (vec![], 0, None);
    let flush =
        |out: &mut Vec<Message>, texts: &mut Vec<String>, tools: &mut u32, ts: &mut Option<i64>| {
            if !texts.is_empty() || *tools > 0 {
                out.push(Message::new("assistant", &texts.join("\n\n"), *ts, *tools));
            }
            *texts = vec![];
            *tools = 0;
            *ts = None;
        };
    each_step(path, 0, |s| match s.kind.as_str() {
        "USER_INPUT" => {
            if let Some(p) = prompt_text(&s.content) {
                flush(&mut out, &mut texts, &mut tools, &mut ts);
                out.push(Message::new("user", &p, s.ts, 0));
            }
        }
        "PLANNER_RESPONSE" => {
            if !s.content.trim().is_empty() {
                texts.push(s.content.trim().to_string());
            }
            tools += s.tools.len() as u32;
            ts = s.ts.or(ts);
        }
        _ => {}
    })?;
    flush(&mut out, &mut texts, &mut tools, &mut ts);
    Ok(out)
}

/// The last ~30 messages with the tool calls and failures between them.
pub fn tail(path: &Path) -> Result<Vec<TailItem>> {
    let mut items: std::collections::VecDeque<TailItem> = Default::default();
    let mut texts = 0usize;
    each_step(path, 0, |s| {
        let mut new = vec![];
        match s.kind.as_str() {
            "USER_INPUT" => {
                if let Some(p) = prompt_text(&s.content) {
                    new.push(TailItem::Text {
                        role: "user",
                        text: tail_chars(&p, 1_500),
                    });
                }
            }
            "PLANNER_RESPONSE" => {
                if !s.content.trim().is_empty() {
                    new.push(TailItem::Text {
                        role: "assistant",
                        text: tail_chars(s.content.trim(), 1_500),
                    });
                }
                new.extend(s.tools.iter().map(|(n, a)| TailItem::Tool(tool_line(n, a))));
            }
            _ => {
                if let Some(e) = s.error.as_deref().filter(|e| !e.is_empty()) {
                    new.push(TailItem::Tool(format!("error: {}", one_line(e, 160))));
                }
            }
        }
        for it in new {
            if matches!(it, TailItem::Text { .. }) {
                texts += 1;
            }
            items.push_back(it);
        }
        while texts > TAIL_TEXTS || items.len() > TAIL_ITEMS_MAX {
            if let Some(TailItem::Text { .. }) = items.pop_front() {
                texts -= 1;
            }
        }
    })?;
    Ok(items.into())
}

/// The time of every step, prompts marked.
pub fn work_events(path: &Path) -> Result<Vec<WorkEvent>> {
    let mut out = vec![];
    each_step(path, 0, |s| {
        if let (Some(ts), true) = (
            s.ts,
            matches!(
                s.kind.as_str(),
                "USER_INPUT" | "PLANNER_RESPONSE" | "GENERIC"
            ),
        ) {
            let prompt = s.kind == "USER_INPUT" && prompt_text(&s.content).is_some();
            if s.kind != "USER_INPUT" || prompt {
                out.push(WorkEvent { ts, prompt });
            }
        }
    })?;
    Ok(out)
}

/// The token use of every model call of the log.
pub fn usage_msgs(path: &Path, models: &dyn Fn() -> Vec<String>) -> Result<Vec<UsageMsg>> {
    Ok(parse(path, 0, 0, models)?.delta.usage_msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_prompt_is_the_user_request_without_the_metadata() {
        let c = "<USER_REQUEST>\nfix it\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nx\n</ADDITIONAL_METADATA>";
        assert_eq!(prompt_text(c).as_deref(), Some("fix it"));
        assert_eq!(prompt_text("plain").as_deref(), Some("plain"));
        assert_eq!(prompt_text("<USER_REQUEST>\n \n</USER_REQUEST>"), None);
    }

    #[test]
    fn steps_decode_with_tokens_and_tools() {
        let l = json!({"step_index":1,"type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-11-05T14:44:25Z",
            "input_tokens":10,"cache_read_tokens":5,"output_tokens":3,"content":"hi",
            "tool_calls":[{"name":"run_command","args":{"CommandLine":"ls  -la"}}]});
        let s = decode(l.to_string().as_bytes()).unwrap();
        assert_eq!(s.tokens.unwrap().cache_read, 5);
        assert_eq!(s.ts, Some(1_793_889_865_000));
        assert_eq!(tool_line(&s.tools[0].0, &s.tools[0].1), "$ ls -la");
        assert!(decode(b"{not json").is_none());
        let g = decode(
            json!({"type":"GENERIC","content":"x"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert!(g.tokens.is_none());
    }

    #[test]
    fn tool_lines_name_the_first_argument() {
        assert_eq!(
            tool_line("search_web", &json!({"query": "quokka diet"})),
            "search_web quokka diet"
        );
        assert_eq!(tool_line("manage_task", &json!({})), "manage_task");
    }
}
