//! The clock's report: the window's hours, the effect of breaks, the bad band, the verdict and what
//! is true now. It is built from the database on request (a few thousand rows) and is what
//! `GET /api/clock` and `eye clock` show.

use super::bands::{self, Band, Hour, Range};
use super::signals::{self, Data};
use super::verdict::{self, part_of_day, Findings, Verdict};
use crate::agents::Agents;
use crate::config::Config;
use crate::night::clock::{local_date, OffsetFn};
use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

/// The longest window the report can cover, in weeks (the corrections pass looks this far back).
pub const MAX_WEEKS: i64 = 12;
const DAY_MS: i64 = 86_400_000;
const MIN_MS: i64 = 60_000;
/// A hint about time since a break waits until it has been this long.
const HINT_AFTER_MIN: i64 = 60;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Now {
    pub in_bad: bool,
    pub since_break_min: Option<i64>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentRow {
    pub agent: String,
    pub prompts: i64,
    pub corrections: i64,
    /// What the agent's thinking measure is ("signature"), null when it reports none.
    pub think: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub enabled: bool,
    pub llm: bool,
    pub hints: bool,
    pub idle: bool,
    pub from: String,
    pub to: String,
    pub prompts: i64,
    pub corrections: i64,
    /// Prompts in the window a model has not been asked about yet.
    pub pending: i64,
    pub hours: Vec<Hour>,
    pub since_break: Vec<Band>,
    pub bad: Vec<Range>,
    pub verdict: Verdict,
    pub now: Now,
    pub agents: Vec<AgentRow>,
}

impl Report {
    /// The API's JSON (see `docs/api.md`).
    pub fn json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

/// The report for the last `weeks` weeks ending now.
pub fn build(
    conn: &Connection,
    cfg: &Config,
    agents: &Agents,
    now: i64,
    weeks: i64,
    off: OffsetFn,
) -> Result<Report> {
    let c = &cfg.clock;
    let llm = crate::config_clock::ClockCfg::llm_on(cfg);
    let today_start = now - (now + off(now)).rem_euclid(DAY_MS);
    let from_ms = today_start - weeks * 7 * DAY_MS;
    let data = if c.enabled {
        signals::load(conn, from_ms)?
    } else {
        Data::default()
    };
    let pending = if llm {
        signals::pending(conn, from_ms)?
    } else {
        0
    };
    let hours = bands::hours(&data, off);
    let break_ms = i64::from(c.break_minutes) * MIN_MS;
    let idle = if c.idle {
        Some(signals::idle_break_ends(conn, from_ms, break_ms)?)
    } else {
        None
    };
    let breaks = bands::break_ends(&data.prompts, idle.as_deref(), break_ms);
    let since_break = bands::since_break(&data.prompts, &breaks);

    let prompts = hours.iter().map(|h| h.prompts).sum();
    let corrections = hours.iter().map(|h| h.corrections).sum();
    let cand = bands::candidate_hours(&hours);
    let (bad_c, bad_p) = sums(&hours, |h| cand[h.h]);
    let you = bands::passes_gate((bad_c, bad_p), (corrections - bad_c, prompts - bad_p))
        .map(|ratio| (ratio, bad_c));
    let cand_ranges = bands::ranges(&cand);
    let band = cand
        .iter()
        .any(|b| *b)
        .then(|| (&cand_ranges[..], verdict::model_side(&data, off, &cand)));
    let verdict = verdict::decide(&Findings {
        prompts,
        corrections,
        llm,
        you,
        band,
    });
    let bad = if you.is_some() && verdict.kind != verdict::Kind::TooFew {
        cand_ranges.clone()
    } else {
        vec![]
    };
    let hour = bands::local_hour(now, off);
    let in_bad = in_ranges(&bad, hour);
    // A user who has been away for a break's length is starting afresh, whatever the last break was.
    let since_break_min = data.prompts.last().map(|p| {
        if now - p.ts >= break_ms {
            0
        } else {
            bands::since_break_min(&breaks, now).unwrap_or(0)
        }
    });
    let (first, later) = since_break.split_first().expect("four bands");
    let later = later
        .iter()
        .fold((0, 0), |(c, p), b| (c + b.corrections, p + b.prompts));
    let break_effect = bands::passes_gate(later, (first.corrections, first.prompts));
    let hint = c
        .hints
        .then(|| {
            hint(
                hour,
                in_bad.then_some(you).flatten(),
                since_break_min,
                break_effect,
            )
        })
        .flatten();
    let now_state = Now {
        in_bad,
        since_break_min,
        hint,
    };
    Ok(Report {
        enabled: c.enabled,
        llm,
        hints: c.hints,
        idle: c.idle,
        from: local_date(from_ms + 6 * 3_600_000, off),
        to: local_date(now, off),
        prompts,
        corrections,
        pending,
        hours,
        since_break,
        bad,
        verdict,
        now: now_state,
        agents: agent_rows(&data, agents),
    })
}

fn sums(hours: &[Hour], pick: impl Fn(&Hour) -> bool) -> (i64, i64) {
    hours
        .iter()
        .filter(|h| pick(h))
        .fold((0, 0), |(c, p), h| (c + h.corrections, p + h.prompts))
}

/// Is `h` inside a run of hours?
fn in_ranges(ranges: &[Range], h: usize) -> bool {
    ranges.iter().any(|r| {
        if r.from_h < r.to_h {
            (r.from_h..r.to_h).contains(&h)
        } else {
            h >= r.from_h || h < r.to_h
        }
    })
}

/// The one quiet line for this moment: inside the bad band, or an hour or more since a break when
/// that has been shown to matter. None when neither holds.
fn hint(
    hour: usize,
    bad: Option<(f64, i64)>,
    since_break_min: Option<i64>,
    break_effect: Option<f64>,
) -> Option<String> {
    if let Some((r, _)) = bad {
        return Some(format!(
            "{}: your corrections usually run {r:.1}x now. {OFFER}",
            part_of_day(hour)
        ));
    }
    match (
        since_break_min.filter(|m| *m >= HINT_AFTER_MIN),
        break_effect,
    ) {
        (Some(_), Some(r)) => Some(format!(
            "An hour or more since a break: your corrections usually run {r:.1}x now. {OFFER}"
        )),
        _ => None,
    }
}

const OFFER: &str = "Hand this to the night shift? (T)";

fn agent_rows(data: &Data, agents: &Agents) -> Vec<AgentRow> {
    let mut rows: Vec<AgentRow> = vec![];
    for p in &data.prompts {
        let i = match rows.iter().position(|r| r.agent == p.agent) {
            Some(i) => i,
            None => {
                rows.push(AgentRow {
                    agent: p.agent.clone(),
                    prompts: 0,
                    corrections: 0,
                    think: agents.get(&p.agent).and_then(|a| a.think_unit()),
                });
                rows.len() - 1
            }
        };
        rows[i].prompts += 1;
        rows[i].corrections += i64::from(p.correction);
    }
    rows.sort_by(|a, b| b.prompts.cmp(&a.prompts).then(a.agent.cmp(&b.agent)));
    rows
}

#[cfg(test)]
mod tests;
