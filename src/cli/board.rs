//! `eye serve` and `eye board`: the optional web board.

use super::args::BoardCmd;
use super::{open_in_browser, Global};
use crate::config;
use crate::report::{Report, Row};
use crate::server;
use anyhow::Result;

pub fn serve(g: &Global, port: Option<u16>, open: bool) -> Result<()> {
    let env = g.open()?;
    let port = port.unwrap_or(env.cfg.board.port);
    if port == env.cfg.board.port && env.cfg.board.enabled {
        if let Some(pid) = crate::daemon::running_pid(&env.paths) {
            let url = server::board_url(port, &server::ensure_token(&env.paths)?);
            println!("the board already runs in the background (daemon pid {pid}):\n{url}");
            if open {
                open_in_browser(&url);
            }
            return Ok(());
        }
    }
    server::serve(env.paths, env.cfg, port, move |url| {
        println!("board: {url}");
        println!("Ctrl-C stops it");
        if open {
            open_in_browser(url);
        }
        Ok(())
    })
}

pub fn board(g: &Global, sub: Option<BoardCmd>, port: Option<u16>, open: bool) -> Result<()> {
    match sub {
        Some(sub) => switch(g, sub),
        None => {
            let env = g.open()?;
            if !env.cfg.board.enabled {
                eprintln!(
                    "the board is off; turn it on with `{} board on` (or run it once with `{} serve`)",
                    crate::pick::bin_name(),
                    crate::pick::bin_name()
                );
                std::process::exit(1);
            }
            let url = server::board_url(
                port.unwrap_or(env.cfg.board.port),
                &server::ensure_token(&env.paths)?,
            );
            println!("{url}");
            if open {
                open_in_browser(&url);
            }
            Ok(())
        }
    }
}

/// `board on` / `board off`: write the setting and restart the daemon.
fn switch(g: &Global, sub: BoardCmd) -> Result<()> {
    let paths = g.paths()?;
    paths.ensure_home()?;
    let (on, no_service) = match &sub {
        BoardCmd::On { no_service, .. } => (true, *no_service),
        BoardCmd::Off { no_service } => (false, *no_service),
    };
    config::write_board_enabled(&paths.config_path(), on)?;
    let mut o = crate::install::Opts::resolve(&paths, None, None)?;
    o.no_service = no_service;
    let mut report = Report::default();
    let mut open = None;
    match sub {
        BoardCmd::On {
            port,
            open: open_it,
            ..
        } => {
            let cfg = g.config(&paths)?;
            let url = server::board_url(
                port.unwrap_or(cfg.board.port),
                &server::ensure_token(&paths)?,
            );
            report.push(Row::new("board", format!("on: {url}")));
            open = open_it.then_some(url);
        }
        BoardCmd::Off { .. } => report.push(Row::new("board", "off")),
    }
    report.extend(crate::install::restart_daemon(&o));
    print!(
        "{}",
        report.render(
            &format!("agent-eye board {}", if on { "on" } else { "off" }),
            false
        )
    );
    if let Some(url) = open {
        open_in_browser(&url);
    }
    if report.failed {
        std::process::exit(1);
    }
    Ok(())
}
