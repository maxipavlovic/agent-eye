//! `eye idea ...`: add, edit, list and start ideas.

use super::args::IdeaCmd;
use super::Global;
use crate::ideas::{self, Edit, NewIdea};
use crate::util::home_rel;
use anyhow::{Context, Result};
use std::path::PathBuf;

fn canonical(d: &std::path::Path) -> String {
    std::fs::canonicalize(d)
        .unwrap_or_else(|_| d.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub fn run(g: &Global, c: IdeaCmd) -> Result<()> {
    let env = g.open()?;
    let check_agent = |a: &Option<String>| -> Result<()> {
        match a.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
            Some(id) => {
                anyhow::ensure!(
                    env.agents.get(id).is_some(),
                    "the agent '{id}' is not enabled"
                );
                Ok(())
            }
            None => Ok(()),
        }
    };
    match c {
        IdeaCmd::Add {
            title,
            notes,
            prompt,
            dir,
            from_session,
            agent,
        } => {
            check_agent(&agent)?;
            let dir: PathBuf = match dir {
                Some(d) => d,
                None => std::env::current_dir().context("no current folder")?,
            };
            let dir = canonical(&dir);
            let from = match from_session {
                Some(f) => Some(env.db.resolve_id(&f)?),
                None => None,
            };
            let i = ideas::add(
                &env.db,
                &NewIdea {
                    title: &title,
                    notes: notes.as_deref(),
                    prompt: prompt.as_deref(),
                    folder: Some(&dir),
                    from_session: from.as_deref(),
                    agent: agent.as_deref(),
                },
            )?;
            println!("idea {} added: {} [{}]", i.id, i.title, home_rel(&dir));
        }
        IdeaCmd::Edit {
            id,
            title,
            notes,
            prompt,
            dir,
            agent,
        } => {
            check_agent(&agent)?;
            let e = Edit {
                title,
                notes,
                prompt,
                folder: dir.as_deref().map(canonical),
                agent,
            };
            let i = ideas::edit(&env.db, id, &e)?;
            println!(
                "idea {} saved: {} [{}]",
                i.id,
                i.title,
                i.folder.as_deref().map(home_rel).unwrap_or_default()
            );
        }
        IdeaCmd::Ls { json } => {
            let all = ideas::list_all(&env.db)?;
            if json {
                println!("{}", serde_json::to_string(&all)?);
            } else if all.is_empty() {
                println!("no ideas");
            }
            for i in all.iter().filter(|_| !json) {
                println!(
                    "{:>3}  {:<8}  {}  [{}]",
                    i.id,
                    if i.session_id.is_some() {
                        "started"
                    } else {
                        "open"
                    },
                    i.title,
                    i.folder.as_deref().map(home_rel).unwrap_or_default()
                );
            }
        }
        IdeaCmd::Start { id, bg, dry_run } => {
            let r = ideas::start(&env.db, &env.cfg, id, bg, dry_run, true)?;
            if r.dry_run {
                println!("{}", r.command);
                if let Some(a) = &r.applescript {
                    println!("--- osascript ---\n{a}");
                }
            } else if r.background {
                println!("{}", r.output);
                match &r.session_id {
                    Some(s) => println!("session {s}"),
                    None => println!("linked to the idea once the job is registered"),
                }
            } else {
                println!(
                    "started session {} ({})",
                    r.session_id.as_deref().unwrap_or("?"),
                    r.mode.as_str()
                );
            }
        }
    }
    Ok(())
}
