//! Whether a bad stretch is you or the model: the model's side measured in the same hours as the
//! user's, and the plain-English line that says so, with the counts.

use super::bands::{local_hour, Range};
use super::signals::Data;
use crate::night::clock::OffsetFn;
use serde::Serialize;
use std::collections::HashMap;

/// The window needs this many prompts and corrections before it is judged.
pub const MIN_PROMPTS: i64 = 300;
pub const MIN_CORRECTIONS: i64 = 30;
/// The model's thinking moved when its ratio leaves this range...
const THINK_FLAT: (f64, f64) = (0.9, 1.1);
/// ...and its failed tool calls when theirs leave this one.
const ERRORS_FLAT: (f64, f64) = (0.8, 1.25);

/// How the model behaved in some hours compared with the other hours. None: nothing to compare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelSide {
    /// Median thinking in the hours over the median elsewhere, each agent's values taken against
    /// that agent's own median first.
    pub think_ratio: Option<f64>,
    /// Failed tool calls per call in the hours over the same elsewhere.
    pub tool_error_ratio: Option<f64>,
}

impl ModelSide {
    pub fn moved(&self) -> bool {
        let out = |r: Option<f64>, (lo, hi): (f64, f64)| r.is_some_and(|r| r < lo || r > hi);
        out(self.think_ratio, THINK_FLAT) || out(self.tool_error_ratio, ERRORS_FLAT)
    }
}

fn median_f(v: &mut [f64]) -> Option<f64> {
    v.sort_by(f64::total_cmp);
    (!v.is_empty()).then(|| v[v.len() / 2])
}

/// The model's side inside the hours set in `mask` against the rest.
pub fn model_side(data: &Data, off: OffsetFn, mask: &[bool]) -> ModelSide {
    let inside = |ts: i64| mask[local_hour(ts, off)];
    let mut own: HashMap<&str, Vec<i64>> = HashMap::new();
    for t in &data.thinks {
        own.entry(&t.agent).or_default().push(t.n);
    }
    let own: HashMap<&str, f64> = own
        .into_iter()
        .filter_map(|(a, mut v)| {
            v.sort_unstable();
            let m = v[v.len() / 2];
            (m > 0).then_some((a, m as f64))
        })
        .collect();
    let (mut bad, mut rest) = (vec![], vec![]);
    for t in &data.thinks {
        if let Some(m) = own.get(t.agent.as_str()) {
            let v = t.n as f64 / m;
            if inside(t.ts) {
                bad.push(v);
            } else {
                rest.push(v);
            }
        }
    }
    let think_ratio = match (median_f(&mut bad), median_f(&mut rest)) {
        (Some(b), Some(r)) if r > 0.0 => Some(b / r),
        _ => None,
    };
    let count = |ts: &[i64]| {
        let n = ts.iter().filter(|t| inside(**t)).count() as f64;
        (n, ts.len() as f64 - n)
    };
    let ((calls_in, calls_out), (err_in, err_out)) = (count(&data.tools), count(&data.tool_errors));
    let tool_error_ratio = (calls_in > 0.0 && calls_out > 0.0 && err_out > 0.0)
        .then(|| (err_in / calls_in) / (err_out / calls_out));
    ModelSide {
        think_ratio,
        tool_error_ratio,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    You,
    Model,
    Both,
    None,
    TooFew,
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
    /// Corrections are being judged (`[clock] llm`).
    pub llm: bool,
    /// The bad band passed the gate: how much more it is corrected, and its corrections.
    pub you: Option<(f64, i64)>,
    /// The hours that stood out (before the gate), if any, and the model's side in them.
    pub band: Option<(&'a [Range], ModelSide)>,
}

/// The time of day a run of hours starts in.
pub fn part_of_day(h: usize) -> &'static str {
    match h {
        5..=11 => "Morning",
        12..=16 => "Afternoon",
        17..=21 => "Evening",
        _ => "Night",
    }
}

fn label(ranges: &[Range]) -> String {
    match ranges {
        [r] => format!("{}s", part_of_day(r.from_h)),
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
fn change(r: f64) -> String {
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
        v.text = if f.llm {
            format!(
                "Too few to tell yet: {} prompts and {} corrections (it needs {MIN_PROMPTS} and {MIN_CORRECTIONS}).",
                f.prompts, f.corrections
            )
        } else {
            "Corrections are not being judged ([clock] llm is off), so there is nothing to compare."
                .into()
        };
        return v;
    }
    let model = f.band.map(|(_, m)| m);
    v.kind = match (f.you.is_some(), model.is_some_and(|m| m.moved())) {
        (true, false) => Kind::You,
        (false, true) => Kind::Model,
        (true, true) => Kind::Both,
        (false, false) => Kind::None,
    };
    let Some((ranges, m)) = f.band.filter(|_| v.kind != Kind::None) else {
        v.text = format!(
            "No clear pattern in {} prompts and {} corrections: no hours stand out clearly and the model's side is steady.",
            f.prompts, f.corrections
        );
        return v;
    };
    v.you_ratio = f.you.map(|(r, _)| round(r, 1));
    v.you_n = f.you.map(|(_, n)| n);
    v.think_ratio = m.think_ratio.map(|r| round(r, 2));
    v.tool_error_ratio = m.tool_error_ratio.map(|r| round(r, 2));
    let yours = match f.you {
        Some((r, n)) => format!("{:.1}x more corrections ({n})", round(r, 1)),
        None => "your corrections about the same".into(),
    };
    let think = m.think_ratio.map_or("No thinking data".into(), |r| {
        format!("Model thinking {}", change(r))
    });
    let errors = match m.tool_error_ratio {
        Some(r) if r < ERRORS_FLAT.0 || r > ERRORS_FLAT.1 => format!("Tool errors {}", change(r)),
        _ => "Tool errors flat".into(),
    };
    let tail = match v.kind {
        Kind::You => "Mostly you.",
        Kind::Model => "Mostly the model.",
        _ => "Both moved.",
    };
    v.text = format!("{}: {yours}. {think}. {errors}. {tail}", label(ranges));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::signals::Think;

    const H: i64 = 3_600_000;

    fn utc(_: i64) -> i64 {
        0
    }
    fn evening() -> Vec<bool> {
        (0..24).map(|h| (18..24).contains(&h)).collect()
    }
    const EVENING: [Range; 1] = [Range {
        from_h: 18,
        to_h: 0,
    }];
    const FLAT: ModelSide = ModelSide {
        think_ratio: Some(0.95),
        tool_error_ratio: Some(1.0),
    };
    const SHALLOW: ModelSide = ModelSide {
        think_ratio: Some(0.8),
        tool_error_ratio: Some(1.0),
    };

    fn kind(prompts: i64, corrections: i64, you: bool, band: Option<ModelSide>) -> Verdict {
        decide(&Findings {
            prompts,
            corrections,
            llm: true,
            you: you.then_some((1.8, 105)),
            band: band.as_ref().map(|m| (&EVENING[..], *m)),
        })
    }

    #[test]
    fn the_model_side_compares_each_agent_with_itself() {
        let think = |h: i64, agent: &str, n| Think {
            ts: h * H,
            agent: agent.into(),
            n,
        };
        let d = Data {
            thinks: vec![
                // the claude values are large and the codex ones small: only the ratio counts
                think(19, "claude", 800),
                think(10, "claude", 1000),
                think(11, "claude", 1000),
                think(19, "codex", 8),
                think(10, "codex", 10),
                think(11, "codex", 10),
            ],
            tools: vec![
                19 * H,
                19 * H,
                10 * H,
                10 * H,
                10 * H,
                11 * H,
                11 * H,
                12 * H,
            ],
            tool_errors: vec![19 * H, 10 * H, 11 * H],
            ..Data::default()
        };
        let m = model_side(&d, &utc, &evening());
        assert!((m.think_ratio.unwrap() - 0.8).abs() < 1e-9);
        // 1 of 2 calls against 2 of 6
        assert!((m.tool_error_ratio.unwrap() - 1.5).abs() < 1e-9);
        assert!(m.moved());
        let none = model_side(&Data::default(), &utc, &evening());
        assert_eq!(
            (none.think_ratio, none.tool_error_ratio, none.moved()),
            (None, None, false)
        );
    }

    #[test]
    fn every_kind_of_verdict() {
        assert_eq!(kind(299, 100, true, Some(FLAT)).kind, Kind::TooFew);
        assert_eq!(kind(900, 29, true, Some(FLAT)).kind, Kind::TooFew);
        assert_eq!(kind(900, 90, true, Some(FLAT)).kind, Kind::You);
        assert_eq!(kind(900, 90, false, Some(SHALLOW)).kind, Kind::Model);
        assert_eq!(kind(900, 90, true, Some(SHALLOW)).kind, Kind::Both);
        assert_eq!(kind(900, 90, false, Some(FLAT)).kind, Kind::None);
        assert_eq!(kind(900, 90, false, None).kind, Kind::None);
    }

    #[test]
    fn the_text_has_the_counts_in_plain_words() {
        assert_eq!(
            kind(2231, 205, true, Some(ModelSide { think_ratio: Some(0.92), tool_error_ratio: Some(0.91) })).text,
            "Evenings: 1.8x more corrections (105). Model thinking \u{2212}8%. Tool errors flat. Mostly you."
        );
        let t = kind(
            900,
            90,
            false,
            Some(ModelSide {
                think_ratio: Some(1.0),
                tool_error_ratio: Some(1.5),
            }),
        )
        .text;
        assert_eq!(
            t,
            "Evenings: your corrections about the same. Model thinking unchanged. Tool errors +50%. Mostly the model."
        );
        assert!(kind(900, 90, true, Some(SHALLOW))
            .text
            .ends_with("Both moved."));
        assert_eq!(
            kind(900, 90, false, None).text,
            "No clear pattern in 900 prompts and 90 corrections: no hours stand out clearly and the model's side is steady."
        );
        assert_eq!(
            kind(120, 12, false, None).text,
            "Too few to tell yet: 120 prompts and 12 corrections (it needs 300 and 30)."
        );
    }

    #[test]
    fn with_judging_off_the_text_says_so() {
        let v = decide(&Findings {
            prompts: 900,
            corrections: 0,
            llm: false,
            you: None,
            band: None,
        });
        assert_eq!(v.kind, Kind::TooFew);
        assert!(v.text.contains("llm is off"));
    }

    #[test]
    fn several_runs_are_named_by_their_hours() {
        let two = [
            Range { from_h: 2, to_h: 4 },
            Range {
                from_h: 22,
                to_h: 1,
            },
        ];
        assert_eq!(label(&two), "Hours 02:00-04:00 and 22:00-01:00");
        assert_eq!(
            label(&[Range {
                from_h: 9,
                to_h: 12
            }]),
            "Mornings"
        );
        assert_eq!(
            label(&[Range {
                from_h: 13,
                to_h: 15
            }]),
            "Afternoons"
        );
        assert_eq!(label(&[Range { from_h: 1, to_h: 5 }]), "Nights");
    }
}
