//! The `eye` / `agent-eye` command line: argument definitions here, one module per group of
//! subcommands.

mod args;
mod backup;
mod board;
mod data;
mod ideas;
mod pick;
mod setup;
mod show;

pub use args::{Cli, Cmd};

use crate::agents::{self, Agents};
use crate::columns;
use crate::config::{Config, UserCfg};
use crate::db::Db;
use crate::model::{Column, Session};
use crate::paths::Paths;
use crate::util::{home_rel, now_ms, printable};
use anyhow::Result;
use args::PickArgs;
use clap::Parser;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The global flags: where eye keeps its data, and Claude Code's folder for this run.
pub(crate) struct Global {
    home: Option<PathBuf>,
    claude_dir: Option<PathBuf>,
}

/// Everything most subcommands need: paths, config, the database and the enabled agents.
pub(crate) struct Env {
    pub paths: Paths,
    pub cfg: Config,
    pub db: Db,
    pub agents: Agents,
}

impl Global {
    pub fn paths(&self) -> Result<Paths> {
        Paths::resolve(self.home.clone())
    }

    /// The config as written (created from the defaults when missing), with the command-line
    /// folder for Claude Code applied.
    pub fn config(&self, paths: &Paths) -> Result<Config> {
        let mut cfg = Config::load_or_create(&paths.config_path())?;
        if let Some(d) = &self.claude_dir {
            cfg.override_agent_dir(agents::LEGACY_ID, d.clone());
        }
        Ok(cfg)
    }

    /// The config without creating it (for commands that must not write: install, doctor).
    pub fn config_read_only(&self, paths: &Paths) -> Config {
        let mut cfg = crate::install::read_config(paths);
        if let Some(d) = &self.claude_dir {
            cfg.override_agent_dir(agents::LEGACY_ID, d.clone());
        }
        cfg
    }

    /// Open the eye home (created 0700), the config and the database.
    pub fn open(&self) -> Result<Env> {
        let paths = self.paths()?;
        paths.ensure_home()?;
        let cfg = self.config(&paths)?;
        let db = Db::open(&paths.db_path())?;
        let agents = agents::enabled(&cfg);
        Ok(Env {
            paths,
            cfg,
            db,
            agents,
        })
    }
}

/// The folder of a session as people read it (`~` for the home folder).
fn folder(s: &Session) -> String {
    s.cwd
        .as_deref()
        .map(|c| printable(&home_rel(c)))
        .unwrap_or_default()
}

fn session_json(s: &Session, col: Column, user: &UserCfg, agents: &Agents) -> Value {
    json!({
        "id": s.id,
        "agent": s.agent,
        "title": s.title,
        "folder": s.cwd,
        "repo": s.repo_root,
        "branch": s.git_branch,
        "column": col.as_str(),
        "kind": s.kind,
        "started_at": s.started_at,
        "last_activity_at": s.last_activity_at,
        "prompt_count": s.prompt_count,
        "reply_count": s.reply_count,
        "state": s.state_line,
        "next": s.next_line().map(|n| user.as_you(&n)),
        "recap": s.recap,
        "live_status": s.live_status,
        "transcript_state": s.transcript_state,
        "parent_id": s.parent_session_id,
        "resume_cmd": agents.resume_line(s),
        "tokens": s.tokens.as_ref().map(|t| t.to_json()),
    })
}

fn column_of(db: &Db, s: &Session) -> Result<Column> {
    let has = !db.outputs_for(&s.id)?.is_empty();
    Ok(columns::compute(&columns::Input::from_session(
        s,
        has,
        now_ms(),
    )))
}

fn open_in_browser(url: &str) {
    if let Err(e) = crate::platform::open_url(url) {
        eprintln!("eye: could not open the browser: {e:#}");
    }
}

/// Parse the command line and run it.
pub fn run() -> Result<()> {
    let mut cli = Cli::parse();
    let cmd = cli.cmd.take().unwrap_or(Cmd::Pick(PickArgs::default()));
    let g = Global {
        home: cli.home,
        claude_dir: cli.claude_dir,
    };
    match cmd {
        Cmd::Hook { event, agent } => {
            // No db, no config, no output, always success: this runs on the agent's hot path.
            if let (Ok(paths), Some(agent)) = (g.paths(), agents::for_hook(agent.as_deref())) {
                crate::hook::run(&paths, agent.as_ref(), &event, std::io::stdin());
            }
            Ok(())
        }
        Cmd::Pick(a) => pick::run(&g, &a),
        Cmd::Init { shell, key } => {
            print!(
                "{}",
                crate::pick::init_script(&shell, &key, &crate::pick::bin_name())?
            );
            Ok(())
        }
        Cmd::Scan { full, json } => show::scan(&g, full, json),
        Cmd::Find {
            words,
            limit,
            all,
            json,
        } => show::find(&g, &words, limit, all, json),
        Cmd::Ls {
            column,
            all,
            limit,
            json,
        } => show::ls(&g, column.as_deref(), all, limit, json),
        Cmd::Hide { id } => show::hide(&g, &id, true),
        Cmd::Unhide { id } => show::hide(&g, &id, false),
        Cmd::Show {
            id,
            json,
            conversation,
            n,
        } => show::show(&g, &id, json, conversation, n),
        Cmd::Resume { id, dry_run, fork } => show::resume(&g, &id, dry_run, fork),
        Cmd::Summarize {
            ids,
            all,
            dry_run,
            max,
            force,
            print_excerpt,
            json,
        } => data::summarize(&g, ids, all, dry_run, max, force, print_excerpt, json),
        Cmd::Serve { port, open } => board::serve(&g, port, open),
        Cmd::Board { sub, port, open } => board::board(&g, sub, port, open),
        Cmd::Idea(c) => ideas::run(&g, c),
        Cmd::Mark {
            args,
            current,
            next,
        } => data::mark(&g, &args, current, next.as_deref()),
        Cmd::Daemon { port, no_board } => setup::daemon(&g, port, no_board),
        Cmd::Install { a, board } => setup::install(&g, &a, board),
        Cmd::Uninstall { a, purge } => setup::uninstall(&g, &a, purge),
        Cmd::Doctor => setup::doctor(&g),
        Cmd::Archive { id, all } => data::archive(&g, id.as_deref(), all),
        Cmd::Restore { id, yes } => {
            if backup::names_snapshot(&g, &id) {
                backup::restore(&g, &id, yes)
            } else {
                data::restore(&g, &id)
            }
        }
        Cmd::Backup { sub } => backup::run(&g, sub),
    }
}
