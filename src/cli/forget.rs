//! `eye forget <id>... | --folder <path> [--transcript] [--yes] [--dry-run]`.

use super::ask;
use super::Global;
use crate::forget::{self, OwnCopy};
use crate::model::Session;
use crate::util::{printable, short_id};
use anyhow::Result;
use std::path::Path;

pub fn run(
    g: &Global,
    ids: &[String],
    folder: Option<&Path>,
    transcript: bool,
    yes: bool,
    dry_run: bool,
) -> Result<()> {
    anyhow::ensure!(
        folder.is_some() || !ids.is_empty(),
        "give session ids, or --folder <path>"
    );
    let env = g.open()?;
    let sessions = match folder {
        Some(f) => forget::by_folder(&env.db, f)?,
        None => forget::by_ids(&env.db, ids)?,
    };
    if sessions.is_empty() {
        println!("no sessions to forget");
        return Ok(());
    }
    let open = sessions.iter().filter(|s| forget::is_open(s)).count();
    println!(
        "{} session{} to forget:",
        sessions.len(),
        if sessions.len() == 1 { "" } else { "s" }
    );
    for s in &sessions {
        println!("{}", line(s, &env, transcript));
    }
    println!(
        "removed from eye: the session, its search text, its archive copy{}",
        if transcript {
            "; and the agent's own copy"
        } else {
            ""
        }
    );
    if !transcript {
        println!("the agent's own copy stays, so the next scan finds the session again; add --transcript to remove it for good");
    }
    if open > 0 {
        anyhow::bail!(
            "{open} of them {} open now: close {} first",
            if open == 1 { "is" } else { "are" },
            if open == 1 { "it" } else { "them" }
        );
    }
    if dry_run {
        println!("dry run: nothing was changed");
        return Ok(());
    }
    if !yes
        && !ask::confirm(
            "forget them? This cannot be undone.",
            ask::From::Terminal("forget without asking"),
        )?
    {
        println!("nothing was changed");
        return Ok(());
    }
    let done = forget::forget(
        &env.db,
        &env.paths,
        &env.cfg,
        &env.agents,
        &sessions,
        transcript,
    )?;
    println!("snapshot first: {}", crate::util::show_path(&done.snapshot));
    let mut failed = 0;
    for o in &done.outcomes {
        let what = match &o.own {
            OwnCopy::Kept => String::new(),
            OwnCopy::Removed(gone) if gone.is_empty() => ", nothing of the agent's was left".into(),
            OwnCopy::Removed(gone) => format!(", and removed {}", gone.join(", ")),
            OwnCopy::Unsupported(why) => format!(", the agent's own copy stays ({why})"),
            OwnCopy::Failed(e) => {
                failed += 1;
                format!(" NOT forgotten: {e}")
            }
        };
        let verb = if o.forgotten { "forgot" } else { "kept" };
        println!("{verb} {}{what}", short_id(&o.id));
    }
    anyhow::ensure!(failed == 0, "{failed} could not be removed");
    Ok(())
}

fn line(s: &Session, env: &super::Env, transcript: bool) -> String {
    let agent = env.agents.short_name(&s.agent);
    let mut l = format!(
        "  {}  {}  {}  {}",
        short_id(&s.id),
        agent,
        printable(s.title.as_deref().unwrap_or("(untitled)")),
        super::folder(s)
    );
    if forget::is_open(s) {
        l.push_str("  [open now: refused]");
    }
    if transcript {
        if env.agents.of_session(s).is_none() {
            l.push_str("  [agent not enabled: its own copy stays]");
        }
    }
    l
}
