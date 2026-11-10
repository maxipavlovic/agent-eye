//! Subcommands that change or keep session data: summarize, mark, archive, restore.

use super::Global;
use crate::archive;
use crate::mark;
use crate::scan;
use crate::summarize;
use crate::util::{printable, short_id};
use anyhow::{Context, Result};

#[allow(clippy::too_many_arguments)]
pub fn summarize(
    g: &Global,
    ids: Vec<String>,
    all: bool,
    dry_run: bool,
    max: Option<usize>,
    force: bool,
    print_excerpt: bool,
    json: bool,
) -> Result<()> {
    anyhow::ensure!(all || !ids.is_empty(), "give session ids or --all");
    let env = g.open()?;
    let out = summarize::summarize(
        &env.db,
        &env.paths,
        &env.cfg,
        &summarize::Options {
            ids,
            all,
            dry_run,
            max,
            force,
            print_excerpt,
        },
    )?;
    if json {
        println!("{}", serde_json::to_string(&out)?);
    } else {
        for o in &out {
            println!("{}", outcome_line(o));
            if let Some(s) = &o.summary {
                println!("    {s}");
            }
        }
    }
    Ok(())
}

/// `id  title  result (123 B, 45 ms, $0.0123)`; the cost is left out when the agent reports none.
fn outcome_line(o: &summarize::Outcome) -> String {
    let cost = o.cost_usd.map(|c| format!(", ${c:.4}")).unwrap_or_default();
    format!(
        "{}  {}  {} ({} B, {} ms{cost})",
        short_id(&o.id),
        printable(o.title.as_deref().unwrap_or("(untitled)")),
        o.result,
        o.excerpt_bytes,
        o.wall_ms
    )
}

pub fn mark(g: &Global, args: &[String], current: bool, next: Option<&str>) -> Result<()> {
    let mut env = g.open()?;
    let (id, status) = match (current, args) {
        (true, [status]) => {
            let cwd = std::env::current_dir().context("no current folder")?;
            let mut found = mark::current_session(&env.db, &env.agents, &cwd)?;
            if found.is_none() {
                scan::scan(&mut env.db, &env.cfg, false)?;
                found = mark::current_session(&env.db, &env.agents, &cwd)?;
            }
            (found.context("no session found for this folder")?, status)
        }
        (false, [id, status]) => (env.db.resolve_id(id)?, status),
        (true, _) => anyhow::bail!("with --current give just the status"),
        (false, _) => anyhow::bail!("give <id> <status>, or --current <status>"),
    };
    let col = mark::mark(&env.db, &id, status, next)?;
    let s = env.db.session_by_id(&id)?.context("session vanished")?;
    println!(
        "{} -> {}  {}",
        short_id(&id),
        col,
        printable(s.title.as_deref().unwrap_or("(untitled)"))
    );
    Ok(())
}

pub fn archive(g: &Global, id: Option<&str>, all: bool) -> Result<()> {
    anyhow::ensure!(all || id.is_some(), "give a session id or --all");
    let env = g.open()?;
    let ids: Vec<String> = match id {
        Some(i) => vec![env.db.resolve_id(i)?],
        None => env.db.all_sessions()?.into_iter().map(|s| s.id).collect(),
    };
    let (mut copied, mut same, mut none, mut bytes) = (0, 0, 0, 0u64);
    for i in ids {
        if !all
            || env
                .db
                .session_by_id(&i)?
                .is_some_and(|s| s.transcript_state.as_deref() == Some("present"))
        {
            match archive::archive_session(&env.db, &env.paths, &env.agents, &i)? {
                archive::Archived::Copied(n) => {
                    copied += 1;
                    bytes += n;
                }
                archive::Archived::Unchanged => same += 1,
                archive::Archived::NoTranscript => none += 1,
            }
        }
    }
    println!(
        "archived {copied} ({:.1} MB compressed), {same} unchanged, {none} without a transcript",
        bytes as f64 / 1e6
    );
    Ok(())
}

pub fn restore(g: &Global, id: &str) -> Result<()> {
    let env = g.open()?;
    let id = env.db.resolve_id(id)?;
    let p = archive::restore(&env.db, &env.paths, &env.agents, &id)?;
    println!("restored {}", p.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(cost_usd: Option<f64>) -> summarize::Outcome {
        summarize::Outcome {
            id: "0123456789abcdef".into(),
            title: Some("Fix it".into()),
            result: "summarized".into(),
            excerpt_bytes: 2048,
            wall_ms: 3210,
            cost_usd,
            summary: None,
        }
    }

    #[test]
    fn the_cost_is_shown_when_the_agent_reports_one_and_left_out_when_not() {
        assert_eq!(
            outcome_line(&outcome(Some(0.0123))),
            "01234567  Fix it  summarized (2048 B, 3210 ms, $0.0123)"
        );
        assert_eq!(
            outcome_line(&outcome(None)),
            "01234567  Fix it  summarized (2048 B, 3210 ms)"
        );
        let json = serde_json::to_value(outcome(None)).unwrap();
        assert!(json.get("cost_usd").is_none(), "{json}");
    }
}
