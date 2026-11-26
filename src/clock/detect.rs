//! Which prompts were corrections: pushback on something the agent did wrong. A cheap word filter
//! picks the prompts that could be; a model, through the same headless call summaries use, decides
//! each one from the prompt and the end of the reply before it. Nothing here touches the database
//! (see `pass`), so the private eval can run the same code on labelled exchanges.

use crate::agents::{Agent, HeadlessRequest};
use crate::conversation::Message;
use crate::summarize::{hash_hex, mask_secrets};
use crate::util::tail_chars;
use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

/// Longest prompt that can be a correction, in characters.
pub const MAX_PROMPT_CHARS: usize = super::signals::MAX_PROMPT_CHARS as usize;
/// How much of the reply before the prompt the model sees, from its end.
pub const REPLY_CHARS: usize = 700;
/// Exchanges in one call.
pub const BATCH_SIZE: usize = 100;
/// Most bytes of exchanges in one call: they travel as one argument of the agent's command line.
const BATCH_BYTES: usize = 90_000;
const TIMEOUT: Duration = Duration::from_secs(240);

/// The words that make a prompt worth asking about. A prompt with none of them has never been
/// pushback in the labelled data, and most prompts have none. Whole words, any case.
pub const CUES: &str = r"no|not|don'?t|never|stop|again|still|why|wrong|instead|too|shorter|simpler|concise|plain|jargon|remove|less|should|must|i said|we agreed|please|avoid";

static CUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?:{CUES})\b")).expect("static regex"));

const SYSTEM: &str = "You read short exchanges between a user and a coding agent. Each one has \
the end of the agent's last reply and the user's next message. For each exchange, decide whether \
the user is pushing back on something the agent did wrong (a correction), not just giving a new \
instruction, answering a question, or approving. If yes, name the complaint in 3-6 plain words. \
Answer for every exchange, by its number.";

/// Could this prompt be a correction? Short enough, and with one of the cue words.
pub fn is_candidate(prompt: &str) -> bool {
    prompt.chars().count() <= MAX_PROMPT_CHARS && CUE_RE.is_match(prompt)
}

/// A prompt and the end of the reply before it, with secrets masked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub prompt: String,
    pub reply: String,
}

impl Exchange {
    /// What is stored to tell that this exact exchange was judged.
    pub fn hash(&self) -> String {
        hash_hex(&format!("{}\n--\n{}", self.reply, self.prompt))
    }
}

/// The exchange of the prompt sent at `prompt_ts`: the user message with that time, and the agent
/// message just before it (empty when the agent said no words). None when the conversation has no
/// such prompt.
pub fn exchange(messages: &[Message], prompt_ts: i64) -> Option<Exchange> {
    let i = messages
        .iter()
        .position(|m| m.role == "user" && m.ts == Some(prompt_ts))?;
    let reply = match i.checked_sub(1).map(|j| &messages[j]) {
        Some(m) if m.role == "assistant" => tail_chars(&m.text, REPLY_CHARS),
        _ => String::new(),
    };
    Some(Exchange {
        prompt: mask_secrets(&messages[i].text),
        reply: mask_secrets(&reply),
    })
}

/// A model's verdict on one exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    pub correction: bool,
    /// The complaint in a few words; empty when it is not a correction.
    pub what: String,
}

/// Split exchanges into calls: at most `BATCH_SIZE` each, and at most `BATCH_BYTES` of text.
pub fn batches<T>(items: Vec<(T, Exchange)>) -> Vec<Vec<(T, Exchange)>> {
    let mut out: Vec<Vec<(T, Exchange)>> = vec![];
    let mut bytes = 0;
    for it in items {
        let size = it.1.prompt.len() + it.1.reply.len() + 40;
        match out.last_mut() {
            Some(b) if b.len() < BATCH_SIZE && bytes + size <= BATCH_BYTES => {
                b.push(it);
                bytes += size;
            }
            _ => {
                out.push(vec![it]);
                bytes = size;
            }
        }
    }
    out
}

fn input(batch: &[&Exchange]) -> String {
    batch
        .iter()
        .enumerate()
        .map(|(i, e)| {
            format!(
                "[{}]\nAGENT (end of its reply): {}\nUSER: {}\n",
                i + 1,
                if e.reply.is_empty() { "(no text)" } else { &e.reply },
                e.prompt
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn schema(n: usize) -> Value {
    let ids: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
    let one = json!({
        "type": "object",
        "properties": {
            "c": {"type": "string", "enum": ["yes", "no"]},
            "what": {"type": "string", "maxLength": 60}
        },
        "required": ["c", "what"]
    });
    json!({
        "type": "object",
        "properties": ids.iter().map(|i| (i.clone(), one.clone())).collect::<serde_json::Map<_, _>>(),
        "required": ids
    })
}

/// Read the answer: one entry per exchange, None for one the model left out or garbled.
fn parse(output: &Value, n: usize) -> Vec<Option<Judged>> {
    (1..=n)
        .map(|i| {
            let o = output.get(i.to_string())?;
            let c = o.get("c")?.as_str()?;
            let correction = match c {
                "yes" => true,
                "no" => false,
                _ => return None,
            };
            let what = o.get("what").and_then(Value::as_str).unwrap_or("").trim();
            Some(Judged {
                correction,
                what: if correction { what.into() } else { String::new() },
            })
        })
        .collect()
}

/// Ask `agent` about the exchanges, in the order given. One call, which fails as a whole (the CLI is
/// missing, the plan limit is hit, the answer is not JSON); the caller decides what to do then.
pub fn judge(
    agent: &dyn Agent,
    model: &str,
    work_dir: &Path,
    batch: &[&Exchange],
) -> Result<Vec<Option<Judged>>> {
    let reply = agent
        .ask(&HeadlessRequest {
            model,
            system: SYSTEM,
            input: &input(batch),
            schema: &schema(batch.len()),
            work_dir,
            timeout: TIMEOUT,
        })
        .context("asking which prompts were corrections")?;
    Ok(parse(&reply.output, batch.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &'static str, text: &str, ts: i64) -> Message {
        Message::new(role, text, Some(ts), 0)
    }

    #[test]
    fn the_cue_words_pick_candidates_on_whole_words() {
        for yes in [
            "No, do it the other way",
            "that is WRONG",
            "don't add a flag",
            "dont add a flag",
            "i said shorter",
            "We agreed on plain text",
            "why did you rename it?",
            "make it less noisy",
        ] {
            assert!(is_candidate(yes), "{yes}");
        }
        for no in [
            "go ahead",
            "now commit it",
            "nothing else to add",
            "the knowledge base",
            "also run the tests",
        ] {
            assert!(!is_candidate(no), "{no}");
        }
    }

    #[test]
    fn a_long_prompt_is_never_a_candidate() {
        let long = format!("no {}", "x".repeat(MAX_PROMPT_CHARS));
        assert!(!is_candidate(&long));
        assert!(is_candidate(&format!("no {}", "x".repeat(MAX_PROMPT_CHARS - 3))));
    }

    #[test]
    fn the_exchange_is_the_prompt_and_the_end_of_the_reply_before_it() {
        let reply = format!("{} the end", "a".repeat(900));
        let all = [
            msg("user", "first", 1),
            msg("assistant", &reply, 2),
            msg("user", "no, token=supersecret123 again", 3),
            msg("user", "another", 4),
        ];
        let e = exchange(&all, 3).unwrap();
        assert_eq!(e.reply.chars().count(), REPLY_CHARS);
        assert!(e.reply.ends_with("the end"));
        assert!(e.prompt.contains("token=[masked]") && !e.prompt.contains("supersecret"));
        // a prompt that follows another prompt has no reply to show; an unknown time has no prompt
        assert_eq!(exchange(&all, 4).unwrap().reply, "");
        assert_eq!(exchange(&all, 9), None);
        assert_eq!(exchange(&all, 2), None);
    }

    #[test]
    fn the_same_exchange_hashes_the_same_and_a_changed_one_does_not() {
        let e = |p: &str| Exchange { prompt: p.into(), reply: "r".into() };
        assert_eq!(e("a").hash(), e("a").hash());
        assert_ne!(e("a").hash(), e("b").hash());
    }

    #[test]
    fn batches_stop_at_a_hundred_exchanges_or_the_byte_limit() {
        let ex = |n: usize| Exchange { prompt: "p".repeat(n), reply: String::new() };
        let many: Vec<(usize, Exchange)> = (0..250).map(|i| (i, ex(10))).collect();
        let sizes: Vec<usize> = batches(many).iter().map(Vec::len).collect();
        assert_eq!(sizes, [100, 100, 50]);
        let big: Vec<(usize, Exchange)> = (0..5).map(|i| (i, ex(40_000))).collect();
        let b = batches(big);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), [2, 2, 1]);
        assert!(batches::<usize>(vec![]).is_empty());
        // the order is kept
        assert_eq!(b[1][0].0, 2);
    }

    #[test]
    fn the_call_numbers_the_exchanges_and_asks_for_every_one() {
        let (a, b) = (
            Exchange { prompt: "no".into(), reply: "done".into() },
            Exchange { prompt: "ok".into(), reply: String::new() },
        );
        let text = input(&[&a, &b]);
        assert!(text.contains("[1]\nAGENT (end of its reply): done\nUSER: no"));
        assert!(text.contains("[2]\nAGENT (end of its reply): (no text)\nUSER: ok"));
        let s = schema(2);
        assert_eq!(s["required"], json!(["1", "2"]));
        assert_eq!(s["properties"]["2"]["required"], json!(["c", "what"]));
    }

    #[test]
    fn the_answer_is_read_per_exchange_and_gaps_stay_gaps() {
        let out = json!({
            "1": {"c": "yes", "what": " too long "},
            "2": {"c": "no", "what": "ignored"},
            "3": {"c": "maybe", "what": ""},
        });
        let got = parse(&out, 4);
        assert_eq!(got[0], Some(Judged { correction: true, what: "too long".into() }));
        assert_eq!(got[1], Some(Judged { correction: false, what: String::new() }));
        assert_eq!((got[2].clone(), got[3].clone()), (None, None));
    }
}
