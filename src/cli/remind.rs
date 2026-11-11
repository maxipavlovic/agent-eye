//! `eye remind`: set, clear and list "check on this" reminders.

use super::Global;
use crate::db::Db;
use crate::remind::{self, Target};
use crate::util::{now_ms, printable, short_id};
use anyhow::{ensure, Result};

pub fn run(g: &Global, args: &[String], note: Option<&str>, clear: bool) -> Result<()> {
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
