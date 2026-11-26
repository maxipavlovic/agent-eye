//! `eye usage`: what the sessions of the last days used and cost, and where the plan limits stand.

use super::Global;
use crate::usage::{backfill, prices, report, series};
use crate::util::{fmt_tokens, now_ms, printable};
use anyhow::{bail, Result};

pub fn run(g: &Global, days: i64, by: &str, by_day: bool, json: bool) -> Result<()> {
    let Some(by) = report::By::parse(by) else {
        bail!("--by takes project, folder, agent, model or session, not '{by}'");
    };
    if days < 1 {
        bail!("--days takes a number of days from 1");
    }
    let mut env = g.open()?;
    // Sessions read before usage by day existed are counted now, so the numbers are whole.
    backfill::run_all(&mut env.db, &env.paths, &env.agents)?;
    let now = now_ms();
    let r = report::build(
        &env.db.conn,
        &env.cfg.usage,
        &env.agents,
        days,
        by,
        now,
        &crate::worked::summary::local_offset_ms,
    )?;
    let by_day = by_day.then(|| {
        series::build(
            &env.db.conn,
            &env.cfg.usage,
            days,
            report::By::Project,
            now,
            &crate::worked::summary::local_offset_ms,
        )
    });
    if json {
        let mut v = r.json(now);
        if let Some(s) = by_day {
            v["series"] = s?.json();
        }
        println!("{v}");
        return Ok(());
    }
    for l in r.limit_lines(now) {
        println!("{l}");
    }
    if !r.limits.is_empty() {
        println!();
    }
    println!(
        "{} to {} ({} day{}), by {}",
        r.from,
        r.to,
        r.days,
        if r.days == 1 { "" } else { "s" },
        by.name()
    );
    if r.rows.is_empty() {
        println!("no usage in this window");
        println!("{}", prices::LABEL);
        return Ok(());
    }
    if let Some(s) = by_day {
        return print_days(&r, &s?);
    }
    let width = r
        .rows
        .iter()
        .map(|x| display_name(by, x).chars().count())
        .max()
        .unwrap_or(0)
        .clamp(8, 48);
    println!(
        "{:<width$}  {:>8}  {:>8}  {:>8}  {:>10}",
        by.name(),
        "in",
        "out",
        "cache",
        "est. cost"
    );
    let line = |name: &str, s: &report::Sum| {
        println!(
            "{:<width$}  {:>8}  {:>8}  {:>8}  {:>10}",
            crate::util::head_chars(name, width),
            fmt_tokens(s.t[0]),
            fmt_tokens(s.t[1]),
            fmt_tokens(s.t[2] + s.t[3]),
            s.cost_text()
        );
    };
    for row in &r.rows {
        line(&display_name(by, row), &row.sum);
    }
    line("total", &r.totals);
    println!("{}", prices::LABEL);
    if r.totals.unpriced > 0 {
        println!(
            "{} tokens are on models without a price (see [usage.prices] in docs/config.md)",
            fmt_tokens(r.totals.unpriced)
        );
    }
    if r.backfill_left > 0 {
        println!(
            "{} sessions are still being counted in the background",
            r.backfill_left
        );
    }
    Ok(())
}

/// One row per day, the days with no usage included, then the total.
fn print_days(r: &report::Report, s: &series::Series) -> Result<()> {
    println!(
        "{:<10}  {:>8}  {:>8}  {:>8}  {:>10}",
        "day", "in", "out", "cache", "est. cost"
    );
    for (day, sum) in s.day_totals() {
        println!(
            "{day:<10}  {:>8}  {:>8}  {:>8}  {:>10}",
            fmt_tokens(sum.t[0]),
            fmt_tokens(sum.t[1]),
            fmt_tokens(sum.t[2] + sum.t[3]),
            sum.cost_text()
        );
    }
    println!(
        "{:<10}  {:>8}  {:>8}  {:>8}  {:>10}",
        "total",
        fmt_tokens(r.totals.t[0]),
        fmt_tokens(r.totals.t[1]),
        fmt_tokens(r.totals.t[2] + r.totals.t[3]),
        r.totals.cost_text()
    );
    println!("{}", prices::LABEL);
    Ok(())
}

fn display_name(by: report::By, row: &report::Row) -> String {
    let name = printable(&row.name).replace('\n', " ");
    match (by, &row.id) {
        (report::By::Session, Some(id)) => {
            format!("{}  {}", crate::util::short_id(id), name)
        }
        _ => name,
    }
}
