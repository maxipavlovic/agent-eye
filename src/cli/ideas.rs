//! `eye idea ...`: add, edit, list and start ideas.

use super::args::IdeaCmd;
use super::Global;
use crate::folder::FolderMissing;
use crate::ideas::{self, Edit, NewIdea, StartOpts};
use crate::util::home_rel;
use anyhow::{Context, Result};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

fn canonical(d: &std::path::Path) -> String {
    std::fs::canonicalize(d)
        .unwrap_or_else(|_| d.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Ask a yes/no question on the terminal; anything but y or yes is no.
fn confirm(question: &str) -> Result<bool> {
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
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
        IdeaCmd::Start {
            id,
            bg,
            create_folder,
            dry_run,
        } => {
            let mut opts = StartOpts {
                background: bg,
                dry_run,
                wait_for_job: true,
                create_folder,
            };
            let first = ideas::start(&env.db, &env.cfg, &env.paths, id, &opts);
            let r = match first {
                Err(e) => match e.downcast_ref::<FolderMissing>() {
                    Some(FolderMissing(dir)) if !dry_run && std::io::stdin().is_terminal() => {
                        if !confirm(&format!("Folder {dir} does not exist. Create it? [y/N] "))? {
                            anyhow::bail!("not started: the folder {dir} does not exist");
                        }
                        opts.create_folder = true;
                        ideas::start(&env.db, &env.cfg, &env.paths, id, &opts)?
                    }
                    Some(FolderMissing(dir)) => anyhow::bail!(
                        "the folder {dir} does not exist yet; add --create-folder to make it"
                    ),
                    None => return Err(e),
                },
                Ok(r) => r,
            };
            if r.creates_folder && !r.dry_run {
                println!("created the folder");
            }
            if r.dry_run {
                if r.creates_folder {
                    println!("would create the folder first");
                }
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
