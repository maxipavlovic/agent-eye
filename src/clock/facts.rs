//! The few plain facts the report states whatever its verdict: how often you correct, how that
//! moved, what a break does and how often you stop the agent. They are something to go on when no
//! hour stands out, so each one rests on enough prompts, and a difference is called a change only
//! when chance rarely makes one that big (the two-proportion test the break hint uses). The
//! corrections are counted only on days the model has judged in full: a prompt still waiting for
//! the nightly pass is no correction yet, and the pass marks the prompts its word filter drops before
//! it asks about the rest, so a day it left half done would look better than it was.

use super::bands::{self, rate, Band};
use super::signals::{Data, Prompt};
use crate::night::clock::{day_number, OffsetFn};
use std::collections::HashSet;

/// A fact needs this many prompts behind each number it gives.
pub const MIN_PROMPTS: i64 = 100;
const WEEK_MS: i64 = 7 * 86_400_000;

/// What the facts are worked out from.
pub struct Inputs<'a> {
    pub data: &'a Data,
    /// When the corrections are judged at all: without that they stay at zero, and only the facts
    /// that are not about corrections are given.
    pub judged: bool,
    /// The end of every break (`bands::break_ends`).
    pub breaks: &'a [i64],
    pub now: i64,
    pub weeks: i64,
    pub off: OffsetFn<'a>,
    /// Is this agent on, so the pass can still read its sessions? A prompt of an agent that is off
    /// waits for good, and holding its day open would only hide the day.
    pub on: &'a dyn Fn(&str) -> bool,
}

/// The facts, in the order a reader wants them; each is left out when it does not rest on enough.
pub fn facts(i: &Inputs) -> Vec<String> {
    let day = |p: &Prompt| day_number(p.ts, i.off);
    let open: HashSet<i64> = i
        .data
        .prompts
        .iter()
        .filter(|p| p.pending && (i.on)(&p.agent))
        .map(day)
        .collect();
    let judged: Vec<&Prompt> = i
        .data
        .prompts
        .iter()
        .filter(|p| !p.pending && !open.contains(&day(p)))
        .collect();
    let mut out = vec![];
    if i.judged {
        out.extend(average(&judged));
        out.extend(trend(&judged, i.now, i.weeks));
        out.extend(breaks(&bands::since_break(
            judged.iter().copied(),
            i.breaks,
        )));
    }
    out.extend(stops(i.data));
    out
}

/// `(corrections, prompts)`.
fn counts<'a>(prompts: impl Iterator<Item = &'a &'a Prompt>) -> (i64, i64) {
    prompts.fold((0, 0), |(c, n), p| (c + i64::from(p.correction), n + 1))
}

/// How group `a` stands against group `b`, each `(corrections, prompts)`: higher or lower when
/// that is more than chance (`bands::passes_gate`), else None.
fn clearly(a: (i64, i64), b: (i64, i64)) -> Option<std::cmp::Ordering> {
    if bands::passes_gate(a, b).is_some() {
        Some(std::cmp::Ordering::Greater)
    } else if bands::passes_gate(b, a).is_some() {
        Some(std::cmp::Ordering::Less)
    } else {
        None
    }
}

/// Per 100, rounded to a whole number.
fn per_100(c: i64, n: i64) -> i64 {
    (rate(c, n) * 100.0).round() as i64
}

fn average(prompts: &[&Prompt]) -> Option<String> {
    let (c, n) = counts(prompts.iter());
    if n < MIN_PROMPTS {
        return None;
    }
    let one_in = (n as f64 / c.max(1) as f64).round() as i64;
    Some(match c {
        0 => format!("None of your {n} prompts was a correction."),
        _ if one_in >= 2 => format!(
            "You correct about 1 in {one_in} prompts: {} per 100 ({c} of {n}).",
            per_100(c, n)
        ),
        _ => format!(
            "{} of every 100 prompts are corrections ({c} of {n}).",
            per_100(c, n)
        ),
    })
}

fn weeks_text(w: i64) -> String {
    if w == 1 {
        "week".into()
    } else {
        format!("{w} weeks")
    }
}

/// The later half of the window against the earlier one (a window of one week has no halves).
fn trend(prompts: &[&Prompt], now: i64, weeks: i64) -> Option<String> {
    let recent = weeks / 2;
    if recent == 0 {
        return None;
    }
    let mid = now - recent * WEEK_MS;
    let (rc, rn) = counts(prompts.iter().filter(|p| p.ts >= mid));
    let (ec, en) = counts(prompts.iter().filter(|p| p.ts < mid));
    if rn < MIN_PROMPTS || en < MIN_PROMPTS {
        return None;
    }
    let (r, e) = (per_100(rc, rn), per_100(ec, en));
    let (last, before) = (weeks_text(recent), weeks_text(weeks - recent));
    Some(match clearly((rc, rn), (ec, en)) {
        Some(std::cmp::Ordering::Less) => {
            format!("Last {last}: {r} per 100, down from {e} in the {before} before.")
        }
        Some(_) => format!("Last {last}: {r} per 100, up from {e} in the {before} before."),
        None if r == e => {
            format!("Last {last}: {r} per 100, the same as in the {before} before.")
        }
        None => format!(
            "Last {last}: {r} per 100, against {e} in the {before} before: not a clear change yet."
        ),
    })
}

/// An hour or more since a break against the first hour after one: the split and the test of the
/// hint about breaks, on the judged days only.
fn breaks(bands: &[Band]) -> Option<String> {
    let (first, later) = bands.split_first()?;
    let later = later
        .iter()
        .fold((0, 0), |(c, n), b| (c + b.corrections, n + b.prompts));
    if first.prompts < MIN_PROMPTS || later.1 < MIN_PROMPTS {
        return None;
    }
    let (f, l) = (
        per_100(first.corrections, first.prompts),
        per_100(later.0, later.1),
    );
    Some(match clearly(later, (first.corrections, first.prompts)) {
        Some(std::cmp::Ordering::Greater) => format!(
            "Breaks help: an hour or more after one, you correct {l} per 100, against {f} in the first hour."
        ),
        Some(_) => format!(
            "An hour or more after a break you correct {l} per 100, fewer than the {f} of the first hour after one."
        ),
        None => format!(
            "An hour or more after a break you correct {l} per 100, against {f} in the first hour: not a clear difference yet."
        ),
    })
}

/// How often you stopped the agent before it finished.
fn stops(data: &Data) -> Option<String> {
    let (s, n) = (data.interrupts.len() as i64, data.prompts.len() as i64);
    if s == 0 || n < MIN_PROMPTS {
        return None;
    }
    let every = (n as f64 / s as f64).round() as i64;
    Some(match s {
        1 => format!("You stopped the agent mid-answer once in {n} prompts."),
        _ if every >= 2 => {
            format!("You stopped the agent mid-answer {s} times, about once every {every} prompts.")
        }
        _ => format!("You stopped the agent mid-answer {s} times in {n} prompts."),
    })
}

#[cfg(test)]
mod tests;
