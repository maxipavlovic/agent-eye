//! Walks every enabled agent's transcripts, keeps read cursors, and imports history, jobs and
//! live state into the database. All agent-specific reading goes through the [`Agent`] trait.

mod children;
mod history;
mod jira;
mod live;
mod transcript;

use crate::agents::{self, Agent, Delta, LastRead, ParseOpts};
use crate::config::Config;
use crate::db::{cursor_set, meta_set, Db};
use crate::util::now_ms;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

/// Folders that are never a session's output, whatever the agent.
const TEMP_DIRS: [&str; 4] = [
    "/tmp",
    "/private/tmp",
    "/var/folders",
    "/private/var/folders",
];

/// What one scan read.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ScanStats {
    pub files_seen: u64,
    pub files_parsed: u64,
    pub bytes_read: u64,
    pub lines: u64,
    pub history_prompts: u64,
    pub elapsed_ms: u128,
}

/// The nearest folder at or above `cwd` that holds a `.git`.
fn repo_root(cwd: &str, cache: &mut HashMap<String, Option<String>>) -> Option<String> {
    if let Some(r) = cache.get(cwd) {
        return r.clone();
    }
    let r = Path::new(cwd)
        .ancestors()
        .find(|a| a.join(".git").exists())
        .map(|a| a.to_string_lossy().into_owned());
    cache.insert(cwd.to_string(), r.clone());
    r
}

/// Bring the database up to date with every enabled agent. `full` re-reads everything.
pub fn scan(db: &mut Db, cfg: &Config, full: bool) -> Result<ScanStats> {
    let t0 = Instant::now();
    // The configured Jira site backs up keys whose site no link has shown yet.
    let site = cfg
        .outputs
        .jira_url
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    if db.meta_get("jira_host:*")?.as_deref().unwrap_or("") != site {
        db.meta_set("jira_host:*", site)?;
    }
    let agents = agents::enabled(cfg);
    let mut stats = ScanStats::default();
    let mut repo_cache = HashMap::new();
    for agent in agents.iter() {
        scan_agent(db, agent.as_ref(), cfg, full, &mut repo_cache, &mut stats)?;
    }
    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    jira::refresh(&tx, cfg)?;
    crate::ideas::link_sessions(&tx, &agents)?;
    live::close_ended(&tx)?;
    if stats.files_parsed > 0 || stats.history_prompts > 0 || full {
        meta_set(&tx, "last_scan_at", &now_ms().to_string())?;
    }
    tx.commit()?;
    stats.elapsed_ms = t0.elapsed().as_millis();
    Ok(stats)
}

/// Recaps stored before the scanner cleaned them as it stored them get the same cleaning, once per
/// agent (what to take out is the agent's call, see `Agent::clean_recap`).
fn clean_stored_recaps(db: &mut Db, agent: &dyn Agent) -> Result<()> {
    let key = format!("recaps_cleaned:{}", agent.id());
    if db.meta_get(&key)?.is_some() {
        return Ok(());
    }
    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let rows: Vec<(String, String)> = tx
        .prepare("SELECT id, recap FROM session WHERE agent=?1 AND recap IS NOT NULL")?
        .query_map([agent.id()], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, recap) in rows {
        let clean = agent.clean_recap(&recap);
        if clean != recap {
            tx.execute(
                "UPDATE session SET recap=?2 WHERE id=?1",
                params![id, clean],
            )?;
        }
    }
    meta_set(&tx, &key, "1")?;
    tx.commit()?;
    Ok(())
}

/// One agent: its transcripts (from the stored cursors), sub-agents, history, jobs and live state.
fn scan_agent(
    db: &mut Db,
    agent: &dyn Agent,
    cfg: &Config,
    full: bool,
    repo_cache: &mut HashMap<String, Option<String>>,
    stats: &mut ScanStats,
) -> Result<()> {
    clean_stored_recaps(db, agent)?;
    let jobs = agent.jobs();
    let mut token_job: HashMap<String, usize> = HashMap::new();
    for (i, j) in jobs.iter().enumerate() {
        token_job.insert(j.short.clone(), i);
        for s in [&j.session_id, &j.resume_session_id].into_iter().flatten() {
            token_job.insert(s.clone(), i);
        }
    }
    let tokens: HashSet<String> = token_job.keys().cloned().collect();
    let mut ignore_dirs: Vec<String> = TEMP_DIRS.map(String::from).to_vec();
    ignore_dirs.extend(agent.own_dirs());
    ignore_dirs.sort();
    ignore_dirs.dedup();

    let files = agent.transcripts();
    let mut hits: Vec<(String, usize)> = vec![];
    for f in &files {
        stats.files_seen += 1;
        let key = f.path.to_string_lossy().into_owned();
        let cursor = db.cursor_get(&key)?;
        if !full && cursor.as_ref().is_some_and(|c| c.stamp == f.stamp) {
            continue;
        }
        // A first read, a full scan and a transcript that came back (deleted, history-only) read
        // everything. Otherwise the adapter is told where the last read stopped and decides whether
        // it can go on from there.
        let since = match &cursor {
            Some(c) if !full && !revived(&db.conn, &f.session_id)? => Some(LastRead {
                marker: &c.marker,
                stamp: c.stamp,
            }),
            _ => None,
        };
        let last_msg_id: Option<String> = if since.is_none() {
            None
        } else {
            db.conn
                .query_row(
                    "SELECT tok_last_id FROM session WHERE id=?1",
                    [&f.session_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten()
        };
        let carry_key = format!("pending:{key}");
        let carry: Option<Value> = if since.is_none() {
            None
        } else {
            db.meta_get(&carry_key)?
                .and_then(|s| serde_json::from_str(&s).ok())
        };
        let opts = ParseOpts {
            job_tokens: &tokens,
            ignore_dirs: &ignore_dirs,
            since,
            carry: carry.as_ref(),
            last_msg_id: last_msg_id.as_deref(),
        };
        let d: Delta = match agent.parse_transcript(f, &opts) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("warn: {e:#}");
                continue;
            }
        };
        stats.files_parsed += 1;
        stats.bytes_read += d.bytes_read;
        stats.lines += d.lines;
        let reset = since.is_none() || d.replaces;
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transcript::apply(&tx, agent, f, &d, reset, cfg, repo_cache)
            .with_context(|| format!("indexing {}", f.path.display()))?;
        crate::worked::store::record(&tx, &f.session_id, &d.events, reset)?;
        match &d.carry {
            Some(c) => meta_set(&tx, &carry_key, &c.to_string())?,
            None => {
                tx.execute("DELETE FROM meta WHERE key=?1", [&carry_key])?;
            }
        }
        cursor_set(&tx, &key, f.stamp, &d.marker)?;
        tx.commit()?;
        for t in &d.child_hits {
            if let Some(&ji) = token_job.get(t) {
                hits.push((f.session_id.clone(), ji));
            }
        }
    }

    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    for f in &files {
        children::scan(&tx, agent, f, full)?;
    }
    mark_missing(&tx, agent.id(), &files)?;
    tx.commit()?;

    let transcript_ids: HashSet<String> = files.iter().map(|f| f.session_id.clone()).collect();
    stats.history_prompts += history::import(db, agent, cfg, &transcript_ids, full, repo_cache)?;

    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    live::apply_jobs(&tx, agent, &jobs, &hits)?;
    live::apply_live(&tx, agent)?;
    tx.commit()?;
    Ok(())
}

fn revived(conn: &Connection, sid: &str) -> Result<bool> {
    let r: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT kind, transcript_state FROM session WHERE id=?1",
            [sid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(r.is_some_and(|(k, t)| {
        k.as_deref() == Some("history_only") || t.as_deref() == Some("deleted")
    }))
}

/// Sessions of this agent that had a transcript and no longer do: archived when eye holds a copy,
/// else deleted.
fn mark_missing(tx: &Connection, agent_id: &str, files: &[agents::Transcript]) -> Result<()> {
    let present: HashSet<&str> = files.iter().map(|f| f.session_id.as_str()).collect();
    let mut st =
        tx.prepare("SELECT id FROM session WHERE transcript_state='present' AND agent=?1")?;
    let ids: Vec<String> = st
        .query_map([agent_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in ids.iter().filter(|i| !present.contains(i.as_str())) {
        tx.execute(
            "UPDATE session SET transcript_state=CASE WHEN archive_path IS NOT NULL THEN 'archived' ELSE 'deleted' END WHERE id=?1",
            [id],
        )?;
    }
    Ok(())
}
