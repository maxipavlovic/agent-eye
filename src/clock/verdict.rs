//! Whether a bad stretch is you or the model: the model's side measured in the same hours as the
//! user's, and the plain-English line that says so, with the counts.

use super::bands::{self, local_hour, Range};
use super::holm;
use super::permute;
use super::signals::{Call, Data};
use crate::night::clock::OffsetFn;
use serde::Serialize;
use std::collections::BTreeMap;

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
    /// shuffle test over every agent's thoughts, held to the family rate with the rest).
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
    /// Its failed calls differ by more than `ERRORS_FLAT`, on enough errors, and more than chance
    /// across the whole report's tests (`holm`).
    pub tool_errors_moved: bool,
    /// How likely chance is to do as well, when it could be told (it needs a ratio of its own).
    tool_error_p: Option<f64>,
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
            let (turns_in, turns_out) = call_turns(data, agent, &inside);
            let sum =
                |turns: &[(i64, i64)]| turns.iter().fold((0, 0), |a, t| (a.0 + t.0, a.1 + t.1));
            let (errs_in, errs_out) = (sum(&turns_in), sum(&turns_out));
            let ((err_in, calls_in), (err_out, calls_out)) = (errs_in, errs_out);
            let tool_error_ratio = (calls_in > 0 && calls_out > 0 && err_out > 0)
                .then(|| (err_in as f64 / calls_in as f64) / (err_out as f64 / calls_out as f64));
            // an agent that makes tool calls always has this test; one that cannot be told apart
            // counts with a p of 1
            let tool_error_p =
                data.tools.iter().any(|c| c.agent == agent).then(|| {
                    permute::errors_p(&turns_in, &turns_out, permute::SEED).unwrap_or(1.0)
                });
            let tool_errors_moved = tool_error_ratio
                .is_some_and(|r| r < ERRORS_FLAT.0 || r > ERRORS_FLAT.1)
                && enough_errors(errs_in, errs_out);
            AgentSide {
                agent: agent.to_string(),
                think_ratio: think_ratio(data, agent, &inside).map(|t| t.0),
                tool_error_ratio,
                tool_errors_moved,
                tool_error_p,
                calls_in,
            }
        })
        .collect()
}

/// What each prompt turn holds: its `(failed calls, calls)`.
type Turns = Vec<(i64, i64)>;

/// An agent's calls by prompt turn, as `(failed calls, calls)`: those of the turns that began inside
/// the hours and of the rest (a turn begins at its first call). In turn order, so a shuffle with a
/// fixed seed is the same every time.
fn call_turns(data: &Data, agent: &str, inside: &dyn Fn(i64) -> bool) -> (Turns, Turns) {
    // turn -> (first call, failed calls, calls)
    let mut turns: BTreeMap<i64, (i64, i64, i64)> = BTreeMap::new();
    let mut add = |c: &Call, failed: i64, calls: i64| {
        let t = turns.entry(c.turn).or_insert((c.ts, 0, 0));
        t.0 = t.0.min(c.ts);
        t.1 += failed;
        t.2 += calls;
    };
    data.tools
        .iter()
        .filter(|c| c.agent == agent)
        .for_each(|c| add(c, 0, 1));
    data.tool_errors
        .iter()
        .filter(|c| c.agent == agent)
        .for_each(|c| add(c, 1, 0));
    let (mut here, mut elsewhere) = (vec![], vec![]);
    for (ts, failed, calls) in turns.into_values() {
        let side = if inside(ts) {
            &mut here
        } else {
            &mut elsewhere
        };
        side.push((failed, calls));
    }
    (here, elsewhere)
}

/// Is the difference between two groups of calls (`(errors, calls)` each) built on enough errors?
/// The same floor as for the user's side: 10 or more in the group with the higher rate.
fn enough_errors(a: (i64, i64), b: (i64, i64)) -> bool {
    let higher = if bands::rate(a.0, a.1) >= bands::rate(b.0, b.1) {
        a
    } else {
        b
    };
    higher.0 >= bands::MIN_EVENTS
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

/// What the tests of one band found.
pub struct Tested {
    /// Your side passed: the hours are corrected more than chance explains.
    pub you: bool,
    /// The model's side.
    pub model: ModelSide,
    /// Each agent's side, in agent order.
    pub sides: Vec<AgentSide>,
}

/// Every test of the band's hours (your corrections, given as `you_p`, the model's thinking over
/// all agents, each agent's failed calls) held to one family-wise rate, so that random data names
/// a pattern no more often than `bands::MAX_P` however many gates there are (`holm`). The family
/// never depends on how the data looks: a gate whose effect is small, or that cannot be told, is
/// still in it with its own p (1 when there is none). The size of an effect is a second condition
/// for a gate to pass, not a reason to leave it out of the family.
pub fn test_band(data: &Data, off: OffsetFn, mask: &[bool], you_p: Option<f64>) -> Tested {
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
    let think_p = think_ratio
        .and_then(|_| permute::median_shift_p(&bad, &rest, permute::SEED))
        .unwrap_or(1.0);
    let mut sides = agent_sides(data, off, mask);
    let mut ps = vec![Some(you_p.unwrap_or(1.0)), Some(think_p)];
    ps.extend(sides.iter().map(|s| s.tool_error_p));
    let passed = holm::survivors(&ps);
    for (side, ok) in sides.iter_mut().zip(&passed[2..]) {
        side.tool_errors_moved &= *ok;
    }
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
    let model = ModelSide {
        think_ratio,
        tool_error_ratio,
        think_moved: think_ratio.is_some_and(|r| r < THINK_FLAT.0 || r > THINK_FLAT.1) && passed[1],
        tool_errors_moved: moved.is_some(),
    };
    Tested {
        you: passed[0],
        model,
        sides,
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
