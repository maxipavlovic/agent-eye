//! `eye export` (one portable file, or one session as a page) and `eye import`.

use super::Global;
use crate::install::Opts;
use crate::pack::{self, Format};
use crate::util::show_path;
use anyhow::{bail, Context, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// `to` as the file to write: a folder gets `name` inside it; nothing means the current folder.
fn target(to: Option<&Path>, name: &str) -> Result<PathBuf> {
    let p = match to {
        Some(t) if t.is_dir() => t.join(name),
        Some(t) => t.to_path_buf(),
        None => std::env::current_dir()?.join(name),
    };
    anyhow::ensure!(!p.exists(), "{} already exists", p.display());
    Ok(p)
}

pub fn export(
    g: &Global,
    session: Option<&str>,
    to: Option<&Path>,
    html: bool,
    md: bool,
) -> Result<()> {
    let env = g.open()?;
    let Some(id) = session else {
        if html || md {
            bail!("--html and --md write one session: give its id (`eye export <session> --html`)");
        }
        let name = pack::pack_name(&pack::host_name(), &jiff::Zoned::now());
        let out = target(to, &name)?;
        let done = pack::export(&env.db, &env.paths, &env.agents, &out)?;
        let c = &done.counts;
        println!("{}", show_path(&done.path));
        println!(
            "{:.1} MB: {} sessions, {} ideas, {} archive files, {} transcripts copied from the agents' folders",
            done.bytes as f64 / 1e6,
            c.sessions,
            c.ideas,
            c.archive_files,
            c.live_files
        );
        println!("it holds your transcripts: keep it private (see docs/security.md)");
        return Ok(());
    };
    let format = if md { Format::Markdown } else { Format::Html };
    let (name, text) = pack::export_session(&env.db, &env.paths, &env.agents, id, format)?;
    let out = target(to, &name)?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&out)
        .with_context(|| format!("creating {}", out.display()))?;
    f.write_all(text.as_bytes())?;
    println!("{} ({:.1} KB)", show_path(&out), text.len() as f64 / 1e3);
    Ok(())
}

pub fn import(g: &Global, file: &Path, yes: bool, dry_run: bool) -> Result<()> {
    let paths = g.paths()?;
    let cfg = g.config_read_only(&paths);
    let plan = pack::plan(&paths, file)?;
    println!("import {}", show_path(file));
    for l in plan.lines(&paths) {
        println!("{l}");
    }
    if dry_run {
        println!("dry run: nothing was changed");
        return Ok(());
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("not a terminal: add --yes to import without asking");
        }
        print!("go on? [y/N] ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut line)
            .context("reading the answer")?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("nothing was changed");
            return Ok(());
        }
    }
    let o = Opts::resolve(&paths, None, None)?;
    let done = pack::apply(&o, &cfg, plan)?;
    match &done.applied.pre_restore {
        Some(p) => println!("kept the old database as {}", show_path(p)),
        None => println!("there was no sound database to keep"),
    }
    if done.applied.daemon_restarted {
        println!("daemon started again");
    }
    if let Some(why) = &done.applied.restart_failed {
        println!("the daemon is not running: {why}");
    }
    println!(
        "imported: {} new and {} updated sessions, {} ideas, {} transcript copies",
        done.sessions_added, done.sessions_updated, done.ideas_added, done.files_copied
    );
    Ok(())
}
