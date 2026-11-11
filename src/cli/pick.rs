//! `eye` / `eye pick`: the interactive picker, and its list without a screen.

use super::args::PickArgs;
use super::Global;
use crate::folder::FolderMissing;
use crate::pick::{self, LaunchOpts, Row};
use crate::scan;
use crate::tui;
use crate::util::now_ms;
use anyhow::{Context, Result};

fn launch(env: &super::Env, row: &Row, print: bool, how: LaunchOpts) -> Result<()> {
    if row
        .session
        .as_ref()
        .is_some_and(|s| s.live_status.is_some())
    {
        eprintln!("eye: warning: this session is open in another terminal");
    }
    let l = pick::prepare(&env.db, &env.paths, &env.agents, &env.cfg, row, how).map_err(
        |e| match (e.downcast_ref::<FolderMissing>(), &row.idea) {
            (Some(m), Some(i)) => anyhow::anyhow!(
                "{m}; start it with `eye idea start {} --create-folder` to make it",
                i.id
            ),
            _ => e,
        },
    )?;
    if print {
        println!("{}", l.command);
        Ok(())
    } else {
        Err(pick::exec(&env.db, &l))
    }
}

pub fn run(g: &Global, a: &PickArgs) -> Result<()> {
    let mut env = g.open()?;
    if !a.no_scan {
        let _ = scan::scan(&mut env.db, &env.cfg, false);
    }
    let now = now_ms();
    let idx = pick::Index::load(&env.db, &env.cfg, now)?;
    let query = a.filter.clone().unwrap_or_else(|| a.words.join(" "));
    if a.filter.is_some() || a.first || !tui::have_tty() {
        let res = pick::search(&env.db, &idx, &query, a.all, now);
        if a.first {
            let Some(&i) = res.first() else {
                anyhow::bail!("no session matches");
            };
            return launch(
                &env,
                &idx.rows[i],
                a.print,
                LaunchOpts {
                    fork: a.fork,
                    create_folder: false,
                },
            );
        }
        if res.is_empty() {
            println!("no sessions match");
        }
        for &i in res.iter().take(a.limit) {
            println!("{}", pick::plain_line(&idx.rows[i], now));
        }
        return Ok(());
    }
    let color = std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let super::Env {
        paths,
        cfg,
        db,
        agents,
    } = env;
    let mut app = tui::App::new(db, idx, &query, color, now);
    app.paths = Some(paths.clone());
    app.user = cfg.user.clone();
    app.agents = agents.clone();
    app.all = a.all;
    if a.all {
        app.research();
    }
    let out = tui::run(&mut app)?;
    match out.clone() {
        tui::Outcome::Quit => Ok(()),
        tui::Outcome::Pick(id) | tui::Outcome::Fork(id) => {
            let fork = matches!(out, tui::Outcome::Fork(_));
            let row = app
                .idx
                .rows
                .iter()
                .find(|r| r.id == id)
                .context("row vanished")?
                .clone();
            let env = super::Env {
                paths,
                cfg,
                db: app.db,
                agents,
            };
            // the picker asked before it let an idea through whose folder is missing
            launch(
                &env,
                &row,
                a.print,
                LaunchOpts {
                    fork,
                    create_folder: true,
                },
            )
        }
    }
}
