//! `eye remind`: set, clear and list "check on this" reminders.

use super::Global;
use crate::db::Db;
use crate::remind::{self, notices, Target};
use crate::util::{now_ms, printable, short_id};
use anyhow::{ensure, Result};

pub fn run(g: &Global, args: &[String], note: Option<&str>, clear: bool) -> Result<()> {
    if args.first().is_some_and(|a| a == "notify") {
        return notify(g, &args[1..]);
    }
    let env = g.open()?;
    let now = now_ms();
    let Some((id, words)) = args.split_first() else {
        anyhow::bail!("say which session or idea");
    };
    if id == "ls" && words.is_empty() {
        return print_list(&env.db, now);
    }
    let target = Target::resolve(&env.db, id)?;
    let title = printable(&target.title(&env.db)?);
    if clear {
        ensure!(words.is_empty(), "--clear takes the id and nothing else");
        remind::clear(&env.db, &target)?;
        println!("reminder cleared: {title}");
        return Ok(());
    }
    ensure!(
        !words.is_empty(),
        "say when: tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20 or \"2026-11-20 14:00\""
    );
    let at = remind::parse_local(&words.join(" "), now)?;
    remind::set(&env.db, &target, at, note)?;
    println!("will remind you {}: {title}", remind::label(at, now));
    if !notices::is_on(&env.paths, env.cfg.remind.notify) {
        println!(
            "desktop notices are off: {} remind notify on",
            crate::pick::bin_name()
        );
    }
    Ok(())
}

/// `remind notify [on|off]`: say whether a due reminder shows a desktop notice, or switch it.
fn notify(g: &Global, words: &[String]) -> Result<()> {
    let paths = g.paths()?;
    let loaded = g.config_read_only(&paths).remind.notify;
    match words {
        [] => {}
        [w] if w == "on" || w == "off" => notices::set(&paths, w == "on")?,
        _ => anyhow::bail!("say `notify on`, `notify off`, or `notify` alone to see the state"),
    }
    println!(
        "{}",
        notices::describe(notices::is_on(&paths, loaded), notices::available())
    );
    Ok(())
}

/// Due reminders first, then the coming ones.
fn print_list(db: &Db, now: i64) -> Result<()> {
    let list = remind::list(db, now)?;
    if list.is_empty() {
        println!("no reminders; set one with: eye remind <id> <when>");
    }
    for e in list {
        let id = if e.id.starts_with("idea:") {
            e.id.clone()
        } else {
            short_id(&e.id).to_string()
        };
        let note = e
            .remind
            .note
            .as_deref()
            .map(|n| format!("  ({})", printable(n)))
            .unwrap_or_default();
        println!(
            "{}  {:<16}  {id:<9}  {}{note}",
            if e.remind.due { "DUE" } else { "   " },
            remind::label(e.remind.at, now),
            printable(&e.title),
        );
    }
    Ok(())
}
