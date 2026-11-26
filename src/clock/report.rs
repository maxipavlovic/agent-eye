//! The clock's report: the window's hours, the effect of breaks, the bad band, the verdict and what
//! is true now. It is built from the database on request (a few thousand rows) and is what
//! `GET /api/clock` and `eye clock` show.

use super::bands::{self, Band, Hour, Range};
use super::signals::{self, Data};
use super::verdict::{self, capitalized, range_label, AgentSide, Findings, Verdict};
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

/// What the board and `eye clock` say about this moment, in one line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hint {
    /// What is true now, a sentence.
    pub text: String,
    /// Handing the work to the night shift is possible (the night shift is on): a surface with a
    /// key or a button for it shows it next to the text.
    pub offer: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Now {
    pub in_bad: bool,
    pub since_break_min: Option<i64>,
    pub hint: Option<Hint>,
}

/// A run of bad hours with its name (`verdict::part_of_day` of its first hour).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bad {
    pub from_h: usize,
    pub to_h: usize,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentRow {
    pub agent: String,
    pub prompts: i64,
    pub corrections: i64,
    /// What the agent's thinking measure is ("signature"), null when it reports none or none was
    /// recorded in the window.
    pub think: Option<&'static str>,
    /// What the agent's thinking measure is, whether or not any was recorded in the window.
    pub think_unit: Option<&'static str>,
    /// Its thinking in the bad hours over its own thinking elsewhere; null when it has no thinking
    /// signal, too little of it, or there are no bad hours.
    pub think_ratio: Option<f64>,
    /// Its failed tool calls per call in the bad hours over its own rate elsewhere; null likewise.
    pub tool_error_ratio: Option<f64>,
    /// The turn events the agent can report at all; one it lacks is "no signal", not zero.
    pub signals: Vec<&'static str>,
}

/// Whether corrections are being judged, and if not, why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Judging {
    On,
    /// Switched off in the config (the clock, or `[clock] llm`, or the summaries it follows).
    Off,
    /// On in the config, but no enabled agent can judge.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub enabled: bool,
    pub llm: bool,
    /// `llm` as a state, with the reason when it is not "on" (null when it is).
    pub judging: Judging,
    pub judging_reason: Option<String>,
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
    /// The hours the verdict names, each with its part of the day.
    pub bad: Vec<Bad>,
    /// The agent the hours' `think` values belong to (the one with the most thinking in the
    /// window), null when none thought.
    pub think_agent: Option<String>,
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
    let any_cand = cand.iter().any(|b| *b);
    let band = any_cand.then(|| (&cand_ranges[..], verdict::model_side(&data, off, &cand)));
    let verdict = verdict::decide(&Findings {
        prompts,
        corrections,
        llm,
        you,
        band,
    });
    // the hours the sentence names are the hours the chart draws
    let named = if verdict.kind.names_hours() {
        cand_ranges
    } else {
        vec![]
    };
    let hour = bands::local_hour(now, off);
    let current = named.iter().find(|r| in_range(r, hour));
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
                current.map(range_label).zip(you),
                since_break_min,
                break_effect,
            )
        })
        .flatten()
        .map(|text| Hint {
            text,
            offer: cfg.night.enabled,
        });
    let (judging, judging_reason) = judging(cfg, agents);
    let sides = if any_cand {
        verdict::agent_sides(&data, off, &cand)
    } else {
        vec![]
    };
    let now_state = Now {
        in_bad: current.is_some(),
        since_break_min,
        hint,
    };
    Ok(Report {
        enabled: c.enabled,
        llm,
        judging,
        judging_reason,
        hints: c.hints,
        idle: c.idle,
        from: local_date(from_ms + 6 * 3_600_000, off),
        to: local_date(now, off),
        prompts,
        corrections,
        pending,
        hours,
        since_break,
        bad: named
            .iter()
            .map(|r| Bad {
                from_h: r.from_h,
                to_h: r.to_h,
                label: range_label(r),
            })
            .collect(),
        think_agent: bands::main_think_agent(&data),
        verdict,
        now: now_state,
        agents: agent_rows(&data, agents, &sides),
    })
}

/// Whether corrections are being judged, with the reason when not.
fn judging(cfg: &Config, agents: &Agents) -> (Judging, Option<String>) {
    let c = &cfg.clock;
    if !c.enabled {
        return (
            Judging::Off,
            Some("the steering clock is off ([clock] enabled)".into()),
        );
    }
    if !crate::config_clock::ClockCfg::llm_on(cfg) {
        let why = match c.llm {
            Some(false) => "[clock] llm is off",
            _ => "[summary] enabled is off, and corrections follow it",
        };
        return (Judging::Off, Some(why.into()));
    }
    if agents.summary_agent(cfg).is_none() {
        return (
            Judging::Unavailable,
            Some("no enabled agent can judge corrections".into()),
        );
    }
    (Judging::On, None)
}

/// The hint's text for this moment, for a surface that has no key or button for the offer (the
/// picker). None when hints are off or nothing applies.
pub fn now_hint(
    conn: &Connection,
    cfg: &Config,
    agents: &Agents,
    now: i64,
    off: OffsetFn,
) -> Result<Option<String>> {
    if !cfg.clock.enabled || !cfg.clock.hints {
        return Ok(None);
    }
    Ok(build(conn, cfg, agents, now, 4, off)?
        .now
        .hint
        .map(|h| h.text))
}

fn sums(hours: &[Hour], pick: impl Fn(&Hour) -> bool) -> (i64, i64) {
    hours
        .iter()
        .filter(|h| pick(h))
        .fold((0, 0), |(c, p), h| (c + h.corrections, p + h.prompts))
}

/// Is `h` inside a run of hours?
fn in_range(r: &Range, h: usize) -> bool {
    if r.from_h < r.to_h {
        (r.from_h..r.to_h).contains(&h)
    } else {
        h >= r.from_h || h < r.to_h
    }
}

/// The one quiet line for this moment: inside the bad band (named by its part of the day, with how
/// much more you are corrected there), or an hour or more since a break when that has been shown to
/// matter. None when neither holds.
fn hint(
    bad: Option<(&str, (f64, i64))>,
    since_break_min: Option<i64>,
    break_effect: Option<f64>,
) -> Option<String> {
    if let Some((part, (r, _))) = bad {
        return Some(format!(
            "{}: your corrections usually run {r:.1}x now.",
            capitalized(part)
        ));
    }
    match (
        since_break_min.filter(|m| *m >= HINT_AFTER_MIN),
        break_effect,
    ) {
        (Some(_), Some(r)) => Some(format!(
            "An hour or more since a break: your corrections usually run {r:.1}x now."
        )),
        _ => None,
    }
}

fn agent_rows(data: &Data, agents: &Agents, sides: &[AgentSide]) -> Vec<AgentRow> {
    let mut rows: Vec<AgentRow> = vec![];
    for p in &data.prompts {
        let i = match rows.iter().position(|r| r.agent == p.agent) {
            Some(i) => i,
            None => {
                let agent = agents.get(&p.agent);
                let think_unit = agent.and_then(|a| a.think_unit());
                let side = sides.iter().find(|s| s.agent == p.agent);
                rows.push(AgentRow {
                    agent: p.agent.clone(),
                    prompts: 0,
                    corrections: 0,
                    think: think_unit.filter(|_| data.thinks.iter().any(|t| t.agent == p.agent)),
                    think_unit,
                    think_ratio: side.and_then(|s| s.think_ratio),
                    tool_error_ratio: side.and_then(|s| s.tool_error_ratio),
                    signals: agent.map_or(vec![], |a| {
                        a.turn_kinds().iter().map(|k| k.as_str()).collect()
                    }),
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
