//! Whether a bad stretch is you or the model: the model's side measured in the same hours as the
//! user's, and the plain-English line that says so, with the counts.

use super::bands::{self, local_hour, Range};
use super::permute;
use super::signals::{Call, Data};
use crate::night::clock::OffsetFn;
use serde::Serialize;

/// The window needs this many prompts and corrections before it is judged.
pub const MIN_PROMPTS: i64 = 300;
pub const MIN_CORRECTIONS: i64 = 30;
/// An agent's thinking is compared only when it thought at least this often inside the hours and
/// outside them (and, as for your corrections, only when chance rarely does as well: `permute`).
pub const MIN_THINK_N: usize = 30;
/// The model's thinking moved when its ratio leaves this range...
const THINK_FLAT: (f64, f64) = (0.9, 1.1);
/// ...and its failed tool calls when theirs leave this one (and more than chance explains it).
const ERRORS_FLAT: (f64, f64) = (0.8, 1.25);

/// How the model behaved in some hours compared with the other hours. None: nothing to compare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelSide {
    /// Median thinking in the hours over the median elsewhere, each agent's values taken against
    /// that agent's own median first.
    pub think_ratio: Option<f64>,
    /// Failed tool calls per call in the hours over the same elsewhere.
    pub tool_error_ratio: Option<f64>,
    /// The thinking differs in those hours by more than `THINK_FLAT` and by more than chance (a
    /// shuffle test over every agent's thoughts, as for your corrections).
    pub think_moved: bool,
    /// Some agent's failed calls differ in those hours by more than chance and by more than
    /// `ERRORS_FLAT` (tested per agent, never pooled).
    pub tool_errors_moved: bool,
}

impl ModelSide {
    /// Was anything on the model's side measured?
    pub fn measured(&self) -> bool {
        self.think_ratio.is_some() || self.tool_error_ratio.is_some()
    }

    pub fn moved(&self) -> bool {
        self.think_moved || self.tool_errors_moved
    }
}

/// One agent's side in the hours of a band.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSide {
    pub agent: String,
    pub think_ratio: Option<f64>,
    pub tool_error_ratio: Option<f64>,
    pub tool_errors_moved: bool,
    /// Its calls inside the hours: how much its ratio weighs.
    calls_in: i64,
}

fn median_f(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    (!v.is_empty()).then(|| v[v.len() / 2])
}

/// Each agent's side inside the hours set in `mask` against the rest, in agent order. An agent's
/// failed calls are compared with its own rate elsewhere (agents fail at different rates), and its
/// thinking with its own median, each only when there is enough of it on both sides.
pub fn agent_sides(data: &Data, off: OffsetFn, mask: &[bool]) -> Vec<AgentSide> {
    let inside = |ts: i64| mask[local_hour(ts, off)];
    let mut agents: Vec<&str> = data
        .thinks
        .iter()
        .map(|t| t.agent.as_str())
        .chain(data.tools.iter().map(|c| c.agent.as_str()))
        .collect();
    agents.sort_unstable();
    agents.dedup();
    agents
        .into_iter()
        .map(|agent| {
            let calls = |list: &[Call]| {
                let n = list.iter().filter(|c| c.agent == agent);
                let inn = n.clone().filter(|c| inside(c.ts)).count() as i64;
                (inn, n.count() as i64 - inn)
            };
            let (calls_in, calls_out) = calls(&data.tools);
            let (err_in, err_out) = calls(&data.tool_errors);
            let tool_error_ratio = (calls_in > 0 && calls_out > 0 && err_out > 0)
                .then(|| (err_in as f64 / calls_in as f64) / (err_out as f64 / calls_out as f64));
            let tool_errors_moved = tool_error_ratio
                .is_some_and(|r| r < ERRORS_FLAT.0 || r > ERRORS_FLAT.1)
                && errors_differ((err_in, calls_in), (err_out, calls_out));
            AgentSide {
                agent: agent.to_string(),
                think_ratio: think_ratio(data, agent, &inside).map(|t| t.0),
                tool_error_ratio,
                tool_errors_moved,
                calls_in,
            }
        })
        .collect()
}

/// Do failed calls differ between two groups of calls (`(errors, calls)` each) by more than chance
/// explains? The same test and floor as for the user's side: p under 0.1, and 10 or more errors in
/// the group with the higher rate.
fn errors_differ(a: (i64, i64), b: (i64, i64)) -> bool {
    let higher = if bands::rate(a.0, a.1) >= bands::rate(b.0, b.1) {
        a
    } else {
        b
    };
    higher.0 >= bands::MIN_EVENTS && bands::proportion_p(a, b).is_some_and(|p| p < bands::MAX_P)
}

/// An agent's thinking in the hours over its thinking elsewhere, with its values taken against its
/// own median so agents can be pooled. None without `MIN_THINK_N` thoughts on both sides.
fn think_ratio(
    data: &Data,
    agent: &str,
    inside: &dyn Fn(i64) -> bool,
) -> Option<(f64, Vec<f64>, Vec<f64>)> {
    let mut own: Vec<i64> = data
        .thinks
        .iter()
        .filter(|t| t.agent == agent)
        .map(|t| t.n)
        .collect();
    own.sort_unstable();
    let median = *own.get(own.len() / 2).filter(|m| **m > 0)? as f64;
    let (mut bad, mut rest) = (vec![], vec![]);
    for t in data.thinks.iter().filter(|t| t.agent == agent) {
        let v = t.n as f64 / median;
        if inside(t.ts) {
            bad.push(v);
        } else {
            rest.push(v);
        }
    }
    if bad.len() < MIN_THINK_N || rest.len() < MIN_THINK_N {
        return None;
    }
    let (b, r) = (median_f(&mut bad)?, median_f(&mut rest)?);
    (r > 0.0).then(|| (b / r, bad, rest))
}

/// The model's side inside the hours set in `mask` against the rest, over all agents.
pub fn model_side(data: &Data, off: OffsetFn, mask: &[bool]) -> ModelSide {
    let inside = |ts: i64| mask[local_hour(ts, off)];
    let (mut bad, mut rest) = (vec![], vec![]);
    let mut seen: Vec<&str> = data.thinks.iter().map(|t| t.agent.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    for agent in seen {
        if let Some((_, b, r)) = think_ratio(data, agent, &inside) {
            bad.extend(b);
            rest.extend(r);
        }
    }
    let think_ratio = match (median_f(&mut bad), median_f(&mut rest)) {
        (Some(b), Some(r)) if r > 0.0 => Some(b / r),
        _ => None,
    };
    let think_moved = think_ratio.is_some_and(|r| r < THINK_FLAT.0 || r > THINK_FLAT.1)
        && permute::median_shift_p(&bad, &rest, permute::SEED).is_some_and(|p| p < bands::MAX_P);
    let sides = agent_sides(data, off, mask);
    // the agent whose errors moved and who made most calls speaks for the band; with none that
    // moved, the measured agents' ratios are averaged, weighted by their calls
    let moved = sides
        .iter()
        .filter(|s| s.tool_errors_moved)
        .max_by_key(|s| s.calls_in);
    let tool_error_ratio = match moved {
        Some(s) => s.tool_error_ratio,
        None => {
            let (sum, weight) = sides
                .iter()
                .filter_map(|s| Some((s.tool_error_ratio?, s.calls_in as f64)))
                .fold((0.0, 0.0), |(a, w), (r, n)| (a + r * n, w + n));
            (weight > 0.0).then(|| sum / weight)
        }
    };
    ModelSide {
        think_ratio,
        tool_error_ratio,
        think_moved,
        tool_errors_moved: moved.is_some(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    You,
    Model,
    Both,
    /// The user's side is clear but nothing on the model's side was measured (an agent that reports
    /// neither thinking nor tool results): it cannot be said whose it was.
    Unmeasured,
    None,
    TooFew,
}

impl Kind {
    /// Does the verdict name hours (the bad band the chart draws)?
    pub fn names_hours(self) -> bool {
        !matches!(self, Kind::None | Kind::TooFew)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub kind: Kind,
    pub you_ratio: Option<f64>,
    pub you_n: Option<i64>,
    pub think_ratio: Option<f64>,
    pub tool_error_ratio: Option<f64>,
    pub text: String,
}

/// What the verdict is made from.
pub struct Findings<'a> {
    pub prompts: i64,
    pub corrections: i64,
    /// Why corrections are not being judged (`report::judging`), None when they are.
    pub not_judged: Option<&'a str>,
    /// The bad band passed the gate: how much more it is corrected, and its corrections.
    pub you: Option<(f64, i64)>,
    /// The hours that stood out (before the gate), if any, and the model's side in them.
    pub band: Option<(&'a [Range], ModelSide)>,
}

/// The time of day an hour is in, lower case: morning 05-11, afternoon 12-16, evening 17-21, night
/// 22-04. The one naming of hours: the verdict, the hint and the board's chart all use it.
pub fn part_of_day(h: usize) -> &'static str {
    match h {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=21 => "evening",
        _ => "night",
    }
}

/// How a run of hours is named: by the part of day it starts in.
pub fn range_label(r: &Range) -> &'static str {
    part_of_day(r.from_h)
}

pub fn capitalized(word: &str) -> String {
    let mut c = word.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn label(ranges: &[Range]) -> String {
    match ranges {
        [r] => format!("{}s", capitalized(range_label(r))),
        _ => {
            let spans: Vec<String> = ranges
                .iter()
                .map(|r| format!("{:02}:00-{:02}:00", r.from_h, r.to_h))
                .collect();
            let (last, first) = spans.split_last().expect("several runs");
            format!("Hours {} and {last}", first.join(", "))
        }
    }
}

/// `−8%` or `+12%` for a ratio.
pub fn change(r: f64) -> String {
    let p = ((r - 1.0) * 100.0).round() as i64;
    match p {
        0 => "unchanged".into(),
        p if p < 0 => format!("\u{2212}{}%", -p),
        p => format!("+{p}%"),
    }
}

fn round(x: f64, places: i32) -> f64 {
    let k = 10f64.powi(places);
    (x * k).round() / k
}

pub fn decide(f: &Findings) -> Verdict {
    let mut v = Verdict {
        kind: Kind::TooFew,
        you_ratio: None,
        you_n: None,
        think_ratio: None,
        tool_error_ratio: None,
        text: String::new(),
    };
    if f.prompts < MIN_PROMPTS || f.corrections < MIN_CORRECTIONS {
        v.text = match f.not_judged {
            None => format!(
                "Too few to tell yet: {} prompts and {} corrections (it needs {MIN_PROMPTS} and {MIN_CORRECTIONS}).",
                f.prompts, f.corrections
            ),
            Some(why) => {
                format!("Corrections are not being judged: {why}, so there is nothing to compare.")
            }
        };
        return v;
    }
    let no_pattern = |model: &str| {
        format!(
            "No clear pattern in {} prompts and {} corrections: {model}.",
            f.prompts, f.corrections
        )
    };
    let Some((ranges, m)) = f.band else {
        v.kind = Kind::None;
        v.text = no_pattern("no hours stand out clearly");
        return v;
    };
    v.kind = match (f.you.is_some(), m.moved(), m.measured()) {
        (true, _, false) => Kind::Unmeasured,
        (true, false, true) => Kind::You,
        (false, true, _) => Kind::Model,
        (true, true, true) => Kind::Both,
        (false, false, _) => Kind::None,
    };
    if v.kind == Kind::None {
        v.text = no_pattern(if m.measured() {
            "the hours that stand out are not clearly different and the model's side is steady"
        } else {
            "the hours that stand out are not clearly different and the model's side has no data"
        });
        return v;
    }
    v.you_ratio = f.you.map(|(r, _)| round(r, 1));
    v.you_n = f.you.map(|(_, n)| n);
    v.think_ratio = m.think_ratio.map(|r| round(r, 2));
    v.tool_error_ratio = m.tool_error_ratio.map(|r| round(r, 2));
    let yours = match f.you {
        Some((r, n)) => format!("{:.1}x more corrections ({n})", round(r, 1)),
        None => "your corrections not clearly different".into(),
    };
    let model = if v.kind == Kind::Unmeasured {
        "No model-side data, so it cannot be said whether it was you or the model.".to_string()
    } else {
        let think = match m.think_ratio {
            None => "No thinking data".into(),
            Some(r) if m.think_moved || (THINK_FLAT.0..=THINK_FLAT.1).contains(&r) => {
                format!("Model thinking {}", change(r))
            }
            Some(_) => "Model thinking not clearly different".into(),
        };
        let errors = match m.tool_error_ratio {
            None => "No tool-error data".into(),
            Some(r) if m.tool_errors_moved => format!("Tool errors {}", change(r)),
            Some(r) if (ERRORS_FLAT.0..=ERRORS_FLAT.1).contains(&r) => "Tool errors flat".into(),
            Some(_) => "Tool errors not clearly different".into(),
        };
        let tail = match v.kind {
            Kind::You => "Mostly you.",
            Kind::Model => "Mostly the model.",
            _ => "Both moved.",
        };
        format!("{think}. {errors}. {tail}")
    };
    v.text = format!("{}: {yours}. {model}", label(ranges));
    v
}

#[cfg(test)]
mod tests;
