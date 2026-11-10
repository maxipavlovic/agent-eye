//! Subcommands that set eye up or check it: daemon, install, uninstall, doctor.

use super::args::SetupArgs;
use super::Global;
use crate::agents;
use crate::db::Db;
use crate::install::{self, Opts};
use crate::report::Report;
use anyhow::Result;
use std::path::PathBuf;

pub fn daemon(g: &Global, port: Option<u16>, no_board: bool) -> Result<()> {
    let paths = g.paths()?;
    paths.ensure_home()?;
    let cfg = g.config(&paths)?;
    let port = port.unwrap_or(cfg.board.port);
    crate::daemon::run(paths, cfg, port, !no_board)
}

fn options(
    g: &Global,
    a: &SetupArgs,
) -> Result<(crate::paths::Paths, crate::config::Config, Opts)> {
    let paths = g.paths()?;
    let cfg = g.config_read_only(&paths);
    let mut o = Opts::resolve(&paths, a.bin_dir.clone(), a.service_dir.clone())?;
    o.dry_run = a.dry_run;
    o.no_service = a.no_service;
    Ok((paths, cfg, o))
}

/// Print what an install or an uninstall did, and fail when something it set out to do did not work.
fn print_report(o: &Opts, title: &str, verbose: bool, report: &Report) {
    let title = if o.dry_run {
        format!("{title} (dry run: nothing was changed)")
    } else {
        title.to_string()
    };
    print!("{}", report.render(&title, o.dry_run || verbose));
    if report.failed {
        std::process::exit(1);
    }
}

pub fn install(g: &Global, a: &SetupArgs, board: bool) -> Result<()> {
    let (paths, cfg, mut o) = options(g, a)?;
    o.board = board;
    if !o.dry_run {
        paths.ensure_home()?;
    }
    let config_path = o.eye_home.join("config.toml");
    let candidates = agents::known(&cfg);
    let choice = install::choose_agents(cfg, &candidates, &config_path, o.dry_run);
    let mut report = Report::default();
    report.push(choice.row.clone());
    report.extend(install::install(&o, &agents::enabled(&choice.cfg))?);
    if !o.dry_run {
        install::save_choice(&choice, &config_path)?;
    }
    print_report(&o, "agent-eye install", a.verbose, &report);
    Ok(())
}

pub fn uninstall(g: &Global, a: &SetupArgs, purge: bool) -> Result<()> {
    let (_, cfg, mut o) = options(g, a)?;
    o.purge = purge;
    let report = install::uninstall(&o, &agents::known(&cfg))?;
    print_report(&o, "agent-eye uninstall", a.verbose, &report);
    Ok(())
}

pub fn doctor(g: &Global) -> Result<()> {
    let paths = g.paths()?;
    let cfg = g.config_read_only(&paths);
    let agents = agents::enabled(&cfg);
    let o = Opts::resolve(&paths, None, None)?;
    let res = install::doctor(&o, &paths, &cfg, &agents);
    let warn = res.iter().filter(|c| !c.ok).count();
    for c in &res {
        println!("{}  {}", if c.ok { "OK  " } else { "WARN" }, c.msg);
    }
    if paths.db_path().exists() {
        if let Ok(db) = Db::open(&paths.db_path()) {
            let hidden = crate::hide::hidden_ids(&db, &cfg)?;
            let by_hand = db
                .all_sessions()?
                .iter()
                .filter(|s| s.hidden == Some(true))
                .count();
            println!(
                "OK    {} sessions hidden ({} by the trivial rule, {} by hand)",
                hidden.len(),
                hidden.len().saturating_sub(by_hand),
                by_hand
            );
        }
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell = if shell.ends_with("bash") {
        "bash"
    } else {
        "zsh"
    };
    let rc = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(if shell == "bash" { ".bashrc" } else { ".zshrc" }));
    let has = rc
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| t.contains("eye init"));
    if has {
        println!("OK    shell key is set up in your {shell} rc file (Ctrl-G opens the picker)");
    } else {
        println!(
            "NOTE  for the Ctrl-G picker, add this line to ~/.{shell}rc:  {}",
            crate::pick::rc_line(shell, &crate::pick::bin_name())
        );
    }
    if warn > 0 {
        std::process::exit(1);
    }
    Ok(())
}
