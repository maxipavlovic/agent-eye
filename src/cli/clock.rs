//! `eye clock`: which hours you steer worse, and whether it was you or the model.

use super::args::ClockCmd;
use super::Global;
use crate::clock::bands::{rate, MIN_HOUR_PROMPTS};
use crate::clock::pass;
use crate::clock::report::{self, Report, MAX_WEEKS};
use crate::clock::verdict;
use crate::turns::backfill;
use crate::util::now_ms;
use crate::worked::summary::local_offset_ms;
use anyhow::{bail, Result};

const LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// The longest bar of the time-since-break rows, in cells.
const BAR_CELLS: usize = 24;

pub fn run(g: &Global, sub: Option<ClockCmd>, weeks: i64, json: bool, brief: bool) -> Result<()> {
    if !(1..=MAX_WEEKS).contains(&weeks) {
        bail!("--weeks takes a number from 1 to {MAX_WEEKS}");
    }
    if matches!(sub, Some(ClockCmd::Detect)) {
        return detect(g);
    }
    let mut env = g.open()?;
    // Sessions read before turns were kept get theirs now, so the numbers are whole. A one-line
    // answer for an agent skips it: the daemon does this in the background.
    if !brief {
        backfill::run_all(&mut env.db, &env.paths, &env.cfg)?;
    }
    let r = report::build(
        &env.db.conn,
        &env.cfg,
        &env.agents,
        now_ms(),
        weeks,
        &local_offset_ms,
    )?;
    if json {
        println!("{}", r.json());
    } else if brief {
        println!("{}", brief_line(&r));
    } else {
        print!("{}", text(&r, weeks));
    }
    Ok(())
}

/// Judge the prompts nobody has judged yet, with a progress line per call.
fn detect(g: &Global) -> Result<()> {
    let mut env = g.open()?;
    backfill::run_all(&mut env.db, &env.paths, &env.cfg)?;
    let out = pass::run(
        &env.db,
        &env.paths,
        &env.cfg,
        &env.agents,
        now_ms(),
        &mut |l| eprintln!("{l}"),
    )?;
    println!(
        "{} exchanges judged in {} sessions, {} corrections; {} prompts still wait for a later pass",
        out.sent, out.sessions, out.corrections, out.pending
    );
    if let Some(why) = out.stopped {
        println!("stopped: {why}");
    }
    Ok(())
}

/// The verdict as one line, for people in a hurry and for an agent that wants to know whether this
/// is a bad hour.
fn brief_line(r: &Report) -> String {
    if !r.enabled {
        return "The steering clock is off ([clock] enabled = false).".into();
    }
    if r.now.in_bad {
        format!("{} It is one of those hours now.", r.verdict.text)
    } else {
        r.verdict.text.clone()
    }
}

/// A block for a rate against the highest rate of its strip.
fn level(rate: f64, max: f64) -> char {
    if max <= 0.0 {
        return LEVELS[0];
    }
    LEVELS[((rate / max * 7.0).round() as usize).min(7)]
}

fn per_100(corrections: i64, prompts: i64) -> f64 {
    rate(corrections, prompts) * 100.0
}

fn text(r: &Report, weeks: i64) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    if !r.enabled {
        return "The steering clock is off ([clock] enabled = false).\n".into();
    }
    let _ = writeln!(
        o,
        "Steering clock: the last {weeks} week{}, {} to {}",
        if weeks == 1 { "" } else { "s" },
        r.from,
        r.to
    );
    let _ = writeln!(
        o,
        "{} prompts, {} corrections ({:.1} per 100){}\n",
        r.prompts,
        r.corrections,
        per_100(r.corrections, r.prompts),
        if r.pending > 0 {
            format!(", {} not judged yet", r.pending)
        } else {
            String::new()
        }
    );
    let rates: Vec<Option<f64>> = r
        .hours
        .iter()
        .map(|h| (h.prompts >= MIN_HOUR_PROMPTS).then(|| per_100(h.corrections, h.prompts)))
        .collect();
    let max = rates.iter().flatten().fold(0.0, |a: f64, b| a.max(*b));
    let strip: String = rates
        .iter()
        .map(|x| x.map_or("··".to_string(), |x| level(x, max).to_string().repeat(2)))
        .collect();
    let ruler: String = (0..24).step_by(3).map(|h| format!("{h:<6}")).collect();
    let _ = writeln!(
        o,
        "Corrections per 100 prompts, by local hour (· = under {MIN_HOUR_PROMPTS} prompts)"
    );
    let _ = writeln!(o, "{strip}\n{ruler}");
    if r.bad.is_empty() {
        let _ = writeln!(o, "No hours stand out.\n");
    } else {
        let spans: Vec<String> = r
            .bad
            .iter()
            .map(|b| format!("{:02}:00-{:02}:00", b.from_h, b.to_h))
            .collect();
        let _ = writeln!(o, "Worse hours: {}\n", spans.join(", "));
    }
    let top = r
        .since_break
        .iter()
        .filter(|b| b.prompts > 0)
        .map(|b| per_100(b.corrections, b.prompts))
        .fold(0.0, f64::max);
    let _ = writeln!(o, "Since your last break");
    for b in &r.since_break {
        let v = per_100(b.corrections, b.prompts);
        let cells = if top > 0.0 {
            (v / top * BAR_CELLS as f64).round() as usize
        } else {
            0
        };
        let _ = writeln!(
            o,
            "  {:<5} {:<w$} {:>5.1} per 100  ({} prompts)",
            b.band,
            "█".repeat(cells),
            v,
            b.prompts,
            w = BAR_CELLS
        );
    }
    let _ = writeln!(o, "\n{}", r.verdict.text);
    if !r.agents.is_empty() {
        let _ = writeln!(
            o,
            "\nBy agent, in the worse hours against its own other hours"
        );
        o.push_str(&agent_lines(r));
        o.push('\n');
    }
    if let Some(m) = r.now.since_break_min {
        let _ = writeln!(
            o,
            "Now: {m} min since your last break{}.",
            if r.now.in_bad {
                ", in one of the worse hours"
            } else {
                ""
            }
        );
    }
    if let Some(h) = &r.now.hint {
        let _ = writeln!(o, "{}", h.text);
    }
    if let Some(why) = &r.judging_reason {
        let _ = writeln!(o, "Corrections are not being judged: {why}.");
    }
    o
}

/// One line per agent: its prompts and corrections, and its model side in the worse hours against
/// its own elsewhere. A signal the agent does not have (or has too little of) is a dash.
fn agent_lines(r: &Report) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let width = r.agents.iter().map(|a| a.agent.len()).max().unwrap_or(0);
    let change = |x: Option<f64>| x.map_or("\u{2014}".to_string(), verdict::change);
    for a in &r.agents {
        let _ = writeln!(
            o,
            "  {:<width$}  {} prompts, {} corrections ({:.1} per 100); thinking {}, tool errors {}",
            a.agent,
            a.prompts,
            a.corrections,
            per_100(a.corrections, a.prompts),
            change(a.think_ratio),
            change(a.tool_error_ratio),
        );
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::bands::{Band, Hour};
    use crate::clock::report::{AgentRow, Bad, Judging, Now};
    use crate::clock::verdict::{Kind, Verdict};

    fn report() -> Report {
        let hours = (0..24)
            .map(|h| Hour {
                h,
                prompts: if h < 20 { 100 } else { 5 },
                corrections: if h == 19 { 20 } else { 10 },
                interrupts: 0,
                tool_calls: 0,
                tool_errors: 0,
                think: None,
                think_n: 0,
            })
            .collect();
        Report {
            enabled: true,
            llm: true,
            judging: Judging::On,
            judging_reason: None,
            hints: false,
            idle: false,
            from: "2026-11-03".into(),
            to: "2026-12-01".into(),
            prompts: 2000,
            corrections: 210,
            pending: 12,
            hours,
            since_break: [
                ("<1h", 1000, 70),
                ("1-2h", 500, 50),
                ("2-4h", 0, 0),
                ("4h+", 500, 90),
            ]
            .map(|(band, prompts, corrections)| Band {
                band,
                prompts,
                corrections,
            })
            .into(),
            bad: vec![Bad {
                from_h: 19,
                to_h: 21,
                label: "evening",
            }],
            think_agent: None,
            tool_agent: None,
            verdict: Verdict {
                kind: Kind::You,
                you_ratio: Some(2.0),
                you_n: Some(20),
                think_ratio: Some(1.0),
                tool_error_ratio: Some(1.0),
                text: "Evenings: 2.0x more corrections (20). Mostly you.".into(),
            },
            now: Now {
                in_bad: true,
                since_break_min: Some(35),
                hint: None,
            },
            agents: vec![],
        }
    }

    fn agent(name: &str, think: Option<f64>, errors: Option<f64>) -> AgentRow {
        AgentRow {
            agent: name.into(),
            prompts: 1000,
            corrections: 100,
            think: None,
            think_unit: None,
            think_ratio: think,
            tool_error_ratio: errors,
            signals: vec![],
        }
    }

    #[test]
    fn the_report_reads_top_to_bottom() {
        let t = text(&report(), 4);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(
            lines[0],
            "Steering clock: the last 4 weeks, 2026-11-03 to 2026-12-01"
        );
        assert_eq!(
            lines[1],
            "2000 prompts, 210 corrections (10.5 per 100), 12 not judged yet"
        );
        // hours under 20 prompts are dots, the busiest strip is the tallest block
        assert!(
            lines[4].starts_with("▅▅▅▅") && lines[4].ends_with("██········"),
            "{}",
            lines[4]
        );
        assert!(lines[5].starts_with("0     3     6 "));
        assert_eq!(lines[6], "Worse hours: 19:00-21:00");
        assert!(
            t.contains("  <1h   █████████                  7.0 per 100  (1000 prompts)"),
            "{t}"
        );
        assert!(
            t.contains("  2-4h                             0.0 per 100  (0 prompts)"),
            "{t}"
        );
        assert!(t.contains("Now: 35 min since your last break, in one of the worse hours."));
    }

    #[test]
    fn the_brief_line_is_the_verdict_and_says_when_it_is_such_an_hour() {
        let mut r = report();
        assert!(brief_line(&r).ends_with("Mostly you. It is one of those hours now."));
        r.now.in_bad = false;
        assert_eq!(brief_line(&r), r.verdict.text);
        r.enabled = false;
        assert!(brief_line(&r).contains("is off"));
    }

    #[test]
    fn an_empty_window_has_no_strip_to_draw() {
        let mut r = report();
        for h in &mut r.hours {
            h.prompts = 0;
            h.corrections = 0;
        }
        r.bad.clear();
        let t = text(&r, 1);
        assert!(t.contains("the last 1 week,"));
        assert!(t.contains(&"··".repeat(24)));
        assert!(t.contains("No hours stand out."));
    }

    #[test]
    fn each_agent_gets_a_line_with_a_dash_for_a_missing_signal() {
        let mut r = report();
        r.agents = vec![
            agent("claude", Some(0.92), Some(1.5)),
            agent("agy", None, None),
        ];
        let t = text(&r, 4);
        assert!(
            t.contains("  claude  1000 prompts, 100 corrections (10.0 per 100); thinking \u{2212}8%, tool errors +50%\n"),
            "{t}"
        );
        assert!(
            t.contains("  agy     1000 prompts, 100 corrections (10.0 per 100); thinking \u{2014}, tool errors \u{2014}\n"),
            "{t}"
        );
    }

    #[test]
    fn the_report_says_why_judging_is_off() {
        let mut r = report();
        r.judging = Judging::Unavailable;
        r.judging_reason = Some("no enabled agent can judge corrections".into());
        assert!(text(&r, 4).ends_with(
            "Corrections are not being judged: no enabled agent can judge corrections.\n"
        ));
    }
}
