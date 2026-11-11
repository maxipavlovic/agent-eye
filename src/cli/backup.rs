//! `eye backup`, `eye backup ls` and `eye restore <snapshot>`.

use super::args::BackupCmd;
use super::Global;
use crate::backup::{self, Reason};
use crate::install::Opts;
use crate::util::{ago, now_ms, show_path};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, IsTerminal, Write};

pub fn run(g: &Global, sub: Option<BackupCmd>) -> Result<()> {
    match sub {
        None => now(g),
        Some(BackupCmd::Ls) => ls(g),
    }
}

/// A snapshot by hand, which works whether automatic ones are on or not.
fn now(g: &Global) -> Result<()> {
    let env = g.open()?;
    let snap = backup::take(&env.db.conn, &env.paths, &env.cfg.backup, &Reason::Manual)?;
    println!(
        "snapshot {} ({:.1} MB)",
        show_path(&snap.path),
        snap.bytes as f64 / 1e6
    );
    if !env.cfg.backup.enabled {
        println!("automatic snapshots are off ([backup] enabled = false in config.toml)");
    }
    Ok(())
}

fn ls(g: &Global) -> Result<()> {
    let paths = g.paths()?;
    let all = backup::list(&backup::dir(&paths));
    if all.is_empty() {
        println!("no snapshots yet (`eye backup` makes one)");
        return Ok(());
    }
    let now = now_ms();
    println!(
        "{:<32} {:<12} {:<9} {:>8}  schema",
        "name", "reason", "age", "size"
    );
    for e in &all {
        let age = e.stamp_ms().map_or("?".to_string(), |t| ago(now, t));
        let schema = backup::schema_version(&paths, e).map_or("?".to_string(), |v| v.to_string());
        println!(
            "{:<32} {:<12} {:<9} {:>6.1} MB  {schema}",
            e.name.trim_end_matches(".db.zst"),
            e.reason.label(),
            age,
            e.bytes as f64 / 1e6
        );
    }
    Ok(())
}

/// Is `name` a database snapshot (or `latest`) rather than a session id?
pub fn names_snapshot(g: &Global, name: &str) -> bool {
    g.paths()
        .is_ok_and(|p| backup::is_snapshot(&backup::dir(&p), name))
}

/// Put a snapshot back as the database, after showing what will happen and asking.
pub fn restore(g: &Global, name: &str, yes: bool) -> Result<()> {
    let paths = g.paths()?;
    let cfg = g.config_read_only(&paths);
    let entry = backup::find(&backup::dir(&paths), name)?;
    let prepared = backup::prepare(&paths, entry)?;
    let e = &prepared.entry;
    println!(
        "restore {} (made {}, schema {}, {:.1} MB)",
        e.name,
        e.stamp.format("%Y-%m-%d %H:%M"),
        prepared.schema,
        e.bytes as f64 / 1e6
    );
    println!("  - the current database is kept first as a pre-restore snapshot");
    println!("  - the daemon is stopped while the database is replaced, and started again");
    println!("  - ideas, column moves and summaries made after this snapshot are not in it");
    if !yes && !confirm()? {
        println!("nothing was changed");
        return Ok(());
    }
    let o = Opts::resolve(&paths, None, None)?;
    let done = backup::apply(&o, &cfg, &prepared)?;
    match &done.pre_restore {
        Some(p) => println!("kept the old database as {}", show_path(p)),
        None => println!("there was no sound database to keep"),
    }
    if done.daemon_restarted {
        println!("daemon started again");
    }
    if let Some(why) = &done.restart_failed {
        println!("the daemon is not running: {why}");
    }
    println!("restored; the next scan brings back sessions newer than the snapshot");
    Ok(())
}

/// Ask `[y/N]` on a terminal; a script has to say `--yes`.
fn confirm() -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        bail!("not a terminal: add --yes to restore without asking");
    }
    print!("go on? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the answer")?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}
