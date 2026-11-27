//! Where the corrections fall: by local hour of the day, and by time since the last break. The
//! hours that run clearly worse than the rest, if the difference is more than chance, are the bad
//! band. All times are Unix ms with an offset function for local time, as in `night::clock`.

use super::signals::{Call, Data, Prompt};
use crate::night::clock::OffsetFn;
use serde::Serialize;
use std::collections::HashMap;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const MIN_MS: i64 = 60_000;

/// An hour needs this many prompts before its rate counts.
pub const MIN_HOUR_PROMPTS: i64 = 20;
/// An hour is bad when its correction rate is this many times the median hour's.
const BAD_FACTOR: f64 = 1.5;
/// The band counts only when chance explains the difference less than this often.
pub const MAX_P: f64 = 0.1;
/// A ratio is reported as at most this much (a rest with no corrections at all).
pub const MAX_RATIO: f64 = 99.0;
/// ...and rests on at least this many events (corrections, failed tool calls) in the group that is
/// higher.
pub const MIN_EVENTS: i64 = 10;

/// The labels of the time-since-break bands, shortest first.
pub const BAND_NAMES: [&str; 4] = ["<1h", "1-2h", "2-4h", "4h+"];

/// The local hour (0 to 23) of a moment.
pub fn local_hour(ts: i64, off: OffsetFn) -> usize {
    ((ts + off(ts)).rem_euclid(DAY_MS) / HOUR_MS) as usize
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hour {
    pub h: usize,
    pub prompts: i64,
    pub corrections: i64,
    pub interrupts: i64,
    /// The main tool agent's calls and failed calls (`main_tool_agent`).
    pub tool_calls: i64,
    pub tool_errors: i64,
    /// The median thinking measure of the main agent in this hour (its own unit), if it thought.
    pub think: Option<i64>,
    pub think_n: i64,
}

fn median(v: &mut [i64]) -> Option<i64> {
    v.sort_unstable();
    (!v.is_empty()).then(|| v[v.len() / 2])
}

/// The agent with the most thinking in the data, the one the hours show the median of.
pub fn main_think_agent(data: &Data) -> Option<String> {
    most_common(data.thinks.iter().map(|t| t.agent.as_str()))
}

/// The agent with the most tool calls in the data, the one the hours show the failed calls of (the
/// verdict compares each agent with itself, so the strip may not pool them).
pub fn main_tool_agent(data: &Data) -> Option<String> {
    most_common(data.tools.iter().map(|c| c.agent.as_str()))
}

fn most_common<'a>(agents: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut n: HashMap<&str, usize> = HashMap::new();
    for a in agents {
        *n.entry(a).or_default() += 1;
    }
    n.into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(a, _)| a.to_string())
}

/// The 24 local hours with their counts.
pub fn hours(data: &Data, off: OffsetFn) -> Vec<Hour> {
    let mut hours: Vec<Hour> = (0..24)
        .map(|h| Hour {
            h,
            prompts: 0,
            corrections: 0,
            interrupts: 0,
            tool_calls: 0,
            tool_errors: 0,
            think: None,
            think_n: 0,
        })
        .collect();
    for p in &data.prompts {
        let h = &mut hours[local_hour(p.ts, off)];
        h.prompts += 1;
        h.corrections += i64::from(p.correction);
    }
    for ts in &data.interrupts {
        hours[local_hour(*ts, off)].interrupts += 1;
    }
    let tool_agent = main_tool_agent(data);
    let of_tool_agent = |c: &&Call| tool_agent.as_deref() == Some(c.agent.as_str());
    for c in data.tools.iter().filter(of_tool_agent) {
        hours[local_hour(c.ts, off)].tool_calls += 1;
    }
    for c in data.tool_errors.iter().filter(of_tool_agent) {
        hours[local_hour(c.ts, off)].tool_errors += 1;
    }
    if let Some(main) = main_think_agent(data) {
        let mut by_hour: Vec<Vec<i64>> = vec![vec![]; 24];
        for t in data.thinks.iter().filter(|t| t.agent == main) {
            by_hour[local_hour(t.ts, off)].push(t.n);
        }
        for (h, v) in hours.iter_mut().zip(by_hour.iter_mut()) {
            h.think_n = v.len() as i64;
            h.think = median(v);
        }
    }
    hours
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Band {
    pub band: &'static str,
    pub prompts: i64,
    pub corrections: i64,
}

/// When the user came back from each break, oldest first: the end of every idle span (`idle`, when
/// the sensor is on) and the prompt that ends every gap of `break_ms` between two prompts. Both
/// count, because the daemon samples idle time only while it runs: a night the machine slept
/// leaves no span, but the gap is there. The first prompt starts the first stretch.
pub fn break_ends(prompts: &[Prompt], idle: Option<&[i64]>, break_ms: i64) -> Vec<i64> {
    let mut ends: Vec<i64> = idle.map(<[i64]>::to_vec).unwrap_or_default();
    let mut last: Option<i64> = None;
    for p in prompts {
        if last.is_none_or(|l| p.ts - l >= break_ms) {
            ends.push(p.ts);
        }
        last = Some(p.ts);
    }
    ends.sort_unstable();
    ends.dedup();
    ends
}

/// Minutes since the last break at `ts`, None before the first known break.
pub fn since_break_min(breaks: &[i64], ts: i64) -> Option<i64> {
    let i = breaks.partition_point(|b| *b <= ts);
    (i > 0).then(|| (ts - breaks[i - 1]) / MIN_MS)
}

fn band_of(min: i64) -> usize {
    match min {
        ..=59 => 0,
        60..=119 => 1,
        120..=239 => 2,
        _ => 3,
    }
}

/// Prompts and corrections by time since the last break. Prompts before the first known break
/// (idle data starting mid-window) are left out.
pub fn since_break(prompts: &[Prompt], breaks: &[i64]) -> Vec<Band> {
    let mut bands: Vec<Band> = BAND_NAMES
        .iter()
        .map(|b| Band {
            band: b,
            prompts: 0,
            corrections: 0,
        })
        .collect();
    for p in prompts {
        if let Some(min) = since_break_min(breaks, p.ts) {
            let b = &mut bands[band_of(min)];
            b.prompts += 1;
            b.corrections += i64::from(p.correction);
        }
    }
    bands
}

/// A run of local hours: from `from_h` up to but not including `to_h`, which may be the next day
/// (`18` to `6` is 18:00 to 06:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Range {
    pub from_h: usize,
    pub to_h: usize,
}

/// The runs of hours set in `mask`, wrapping around midnight.
pub fn ranges(mask: &[bool]) -> Vec<Range> {
    let n = mask.len();
    // start counting at an hour that is not set, so a run never straddles the start
    let Some(zero) = mask.iter().position(|m| !m) else {
        return vec![];
    };
    let mut out: Vec<Range> = vec![];
    let mut start: Option<usize> = None;
    for i in 1..=n {
        let h = (zero + i) % n;
        match (mask[h], start) {
            (true, None) => start = Some(h),
            (false, Some(s)) => {
                out.push(Range { from_h: s, to_h: h });
                start = None;
            }
            _ => {}
        }
    }
    out.sort_by_key(|r| r.from_h);
    out
}

/// The hours whose correction rate is at least 1.5 times the median hour's, among the hours with
/// enough prompts. Chance has not been ruled out yet (see `permute::band_test`).
pub fn candidate_hours(hours: &[Hour]) -> Vec<bool> {
    let counts: Vec<(i64, i64)> = hours.iter().map(|h| (h.prompts, h.corrections)).collect();
    let mut mask = vec![false; hours.len()];
    candidate_mask(&counts, &mut mask);
    mask
}

/// `candidate_hours` on `(prompts, corrections)` per hour, written into `mask`.
pub fn candidate_mask(counts: &[(i64, i64)], mask: &mut [bool]) {
    let mut rates: Vec<f64> = counts
        .iter()
        .filter(|(n, _)| *n >= MIN_HOUR_PROMPTS)
        .map(|(n, c)| rate(*c, *n))
        .collect();
    rates.sort_by(f64::total_cmp);
    let mid = rates.get(rates.len() / 2).copied();
    for (m, (n, c)) in mask.iter_mut().zip(counts) {
        let r = rate(*c, *n);
        *m = mid.is_some_and(|mid| *n >= MIN_HOUR_PROMPTS && r > 0.0 && r >= BAD_FACTOR * mid);
    }
}

pub fn rate(corrections: i64, prompts: i64) -> f64 {
    if prompts == 0 {
        0.0
    } else {
        corrections as f64 / prompts as f64
    }
}

/// How much more often the first group is corrected than the second, when that is more than chance
/// and rests on enough corrections: a two-sided two-proportion test under 0.1, with 10 or more
/// corrections in the first group. `(corrections, prompts)` for each. For two groups fixed in
/// advance (before and after a break); groups picked from the data go through `permute`.
pub fn passes_gate(bad: (i64, i64), rest: (i64, i64)) -> Option<f64> {
    let ((c1, n1), (c2, n2)) = (bad, rest);
    if c1 < MIN_EVENTS || rate(c1, n1) <= rate(c2, n2) {
        return None;
    }
    (proportion_p(bad, rest)? < MAX_P).then(|| {
        let (r1, r2) = (rate(c1, n1), rate(c2, n2));
        if r2 == 0.0 {
            MAX_RATIO
        } else {
            (r1 / r2).min(MAX_RATIO)
        }
    })
}

/// The chance that two groups differ in their rate this much (or more) when they do not: a two-sided
/// two-proportion z-test. `(events, size)` for each. None when it cannot be told: a group is empty,
/// or there are no events or only events.
pub fn proportion_p(a: (i64, i64), b: (i64, i64)) -> Option<f64> {
    proportion_z(a, b).map(two_sided_p)
}

/// The two-proportion z statistic (first group over second, signed), None when `proportion_p` is.
pub fn proportion_z(a: (i64, i64), b: (i64, i64)) -> Option<f64> {
    let ((c1, n1), (c2, n2)) = (a, b);
    if n1 == 0 || n2 == 0 {
        return None;
    }
    let pooled = (c1 + c2) as f64 / (n1 + n2) as f64;
    let se = (pooled * (1.0 - pooled) * (1.0 / n1 as f64 + 1.0 / n2 as f64)).sqrt();
    (se > 0.0).then(|| (rate(c1, n1) - rate(c2, n2)) / se)
}

/// P(|Z| > z) for a standard normal Z.
fn two_sided_p(z: f64) -> f64 {
    erfc(z.abs() / std::f64::consts::SQRT_2)
}

/// The complementary error function (Numerical Recipes, fractional error under 1.2e-7).
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.5 * x);
    let poly = -x * x - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    t * poly.exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::signals::{Call, Think};

    const H: i64 = HOUR_MS;

    fn utc(_: i64) -> i64 {
        0
    }
    fn plus2(_: i64) -> i64 {
        2 * H
    }
    fn prompt(ts: i64, correction: bool) -> Prompt {
        Prompt {
            ts,
            agent: "claude".into(),
            correction,
        }
    }
    fn call(ts: i64) -> Call {
        Call {
            ts,
            agent: "claude".into(),
        }
    }
    fn hour(h: usize, prompts: i64, corrections: i64) -> Hour {
        Hour {
            h,
            prompts,
            corrections,
            interrupts: 0,
            tool_calls: 0,
            tool_errors: 0,
            think: None,
            think_n: 0,
        }
    }

    #[test]
    fn hours_are_local() {
        assert_eq!(local_hour(23 * H, &utc), 23);
        assert_eq!(local_hour(23 * H, &plus2), 1);
        assert_eq!(local_hour(-H, &utc), 23);
    }

    #[test]
    fn hours_count_every_signal_and_the_main_agents_thinking() {
        let d = Data {
            prompts: vec![prompt(H, true), prompt(H + 5, false), prompt(5 * H, false)],
            interrupts: vec![H],
            tools: [H, H, 5 * H].map(call).into(),
            tool_errors: vec![call(5 * H)],
            thinks: vec![
                Think {
                    ts: H,
                    agent: "claude".into(),
                    n: 100,
                },
                Think {
                    ts: H,
                    agent: "claude".into(),
                    n: 300,
                },
                Think {
                    ts: H,
                    agent: "claude".into(),
                    n: 200,
                },
                Think {
                    ts: 5 * H,
                    agent: "codex".into(),
                    n: 7,
                },
            ],
        };
        let hs = hours(&d, &utc);
        assert_eq!(hs.len(), 24);
        assert_eq!(
            (hs[1].prompts, hs[1].corrections, hs[1].interrupts),
            (2, 1, 1)
        );
        assert_eq!((hs[1].tool_calls, hs[5].tool_errors), (2, 1));
        assert_eq!((hs[1].think, hs[1].think_n), (Some(200), 3));
        // the other agent's unit is not mixed in
        assert_eq!(hs[5].think, None);
        assert_eq!(hours(&Data::default(), &utc)[0].think, None);
    }

    #[test]
    fn a_break_is_a_gap_between_prompts_or_an_idle_span() {
        let ps: Vec<Prompt> = [0, 10, 20, 100, 110]
            .iter()
            .map(|m| prompt(m * MIN_MS, false))
            .collect();
        let gaps = break_ends(&ps, None, 45 * MIN_MS);
        assert_eq!(gaps, [0, 100 * MIN_MS]);
        // a span the prompts do not show counts too, and a gap counts when no span covers it (the
        // daemon was not running overnight)
        let idle = break_ends(&ps, Some(&[50 * MIN_MS]), 45 * MIN_MS);
        assert_eq!(idle, [0, 50 * MIN_MS, 100 * MIN_MS]);
        let same = break_ends(&ps, Some(&[100 * MIN_MS]), 45 * MIN_MS);
        assert_eq!(same, [0, 100 * MIN_MS]);
        assert!(break_ends(&[], None, 1).is_empty());
    }

    #[test]
    fn since_break_bands_come_from_either_source() {
        let at = |m: i64| prompt(m * MIN_MS, m % 2 == 0);
        let ps: Vec<Prompt> = [0, 30, 59, 60, 130, 250, 400].map(at).into();
        // the same prompts, with a break at 0 from gaps: the gaps are all under the threshold
        let by_gaps = since_break(&ps, &break_ends(&ps, None, 600 * MIN_MS));
        let counts: Vec<_> = by_gaps
            .iter()
            .map(|b| (b.band, b.prompts, b.corrections))
            .collect();
        assert_eq!(
            counts,
            [("<1h", 3, 2), ("1-2h", 1, 1), ("2-4h", 1, 1), ("4h+", 2, 2)]
        );
        // idle data that starts at 100 min leaves the earlier prompts out
        let by_idle = since_break(&ps, &[100 * MIN_MS]);
        assert_eq!(
            by_idle.iter().map(|b| b.prompts).collect::<Vec<_>>(),
            [1, 0, 1, 1]
        );
        assert_eq!(since_break_min(&[100 * MIN_MS], 99 * MIN_MS), None);
        assert_eq!(since_break_min(&[100 * MIN_MS], 100 * MIN_MS), Some(0));
    }

    #[test]
    fn ranges_wrap_around_midnight() {
        let mask = |on: &[usize]| (0..24).map(|h| on.contains(&h)).collect::<Vec<_>>();
        let r = |from_h, to_h| Range { from_h, to_h };
        assert_eq!(
            ranges(&mask(&[18, 19, 20, 21, 22, 23, 0, 1, 2, 3, 4, 5])),
            [r(18, 6)]
        );
        assert_eq!(ranges(&mask(&[9, 10, 11])), [r(9, 12)]);
        assert_eq!(ranges(&mask(&[2, 3, 22, 23, 0])), [r(2, 4), r(22, 1)]);
        assert_eq!(ranges(&mask(&[23])), [r(23, 0)]);
        assert!(ranges(&mask(&[])).is_empty());
    }

    #[test]
    fn bad_hours_need_enough_prompts_and_one_and_a_half_times_the_median_rate() {
        let mut hs: Vec<Hour> = (0..24).map(|h| hour(h, 40, 4)).collect();
        hs[19] = hour(19, 40, 8);
        hs[20] = hour(20, 40, 5);
        hs[21] = hour(21, 10, 9); // a high rate, too few prompts
        hs[3] = hour(3, 0, 0);
        let mask = candidate_hours(&hs);
        let on: Vec<usize> = (0..24).filter(|h| mask[*h]).collect();
        assert_eq!(on, [19]);
        assert!(candidate_hours(&[hour(0, 5, 5)]).iter().all(|b| !b));
    }

    #[test]
    fn the_gate_wants_a_real_difference_and_enough_corrections() {
        // 18% against 9%: clear
        let ratio = passes_gate((54, 300), (180, 2000)).unwrap();
        assert!((ratio - 2.0).abs() < 0.01);
        // the same rates on too few corrections
        assert_eq!(passes_gate((9, 50), (180, 2000)), None);
        // 12 of 100 against 10 of 100 is chance
        assert_eq!(passes_gate((12, 100), (10, 100)), None);
        // a lower rate never passes
        assert_eq!(passes_gate((10, 1000), (50, 100)), None);
        assert_eq!(passes_gate((10, 0), (5, 5)), None);
    }

    #[test]
    fn the_normal_tail_is_right() {
        assert!((two_sided_p(1.645) - 0.1).abs() < 0.001);
        assert!((two_sided_p(1.96) - 0.05).abs() < 0.001);
        assert!((two_sided_p(0.0) - 1.0).abs() < 1e-6);
    }
}
