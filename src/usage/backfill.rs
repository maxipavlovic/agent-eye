//! Sessions read before eye kept usage by day have their cursors at the end of their files, so the
//! normal read never sees their past messages. The backfill reads the token use of those sessions
//! (and of their sub-agents) from the transcript, or from eye's archive copy when the original is
//! gone, newest first, a few per pass, and touches nothing else (no cursor, output, token count or
//! summary). A session is counted once: the daemon does a few per tick, `eye scan` and `eye usage`
//! all of them, and `store::add` leaves a session that is not counted alone, so a pass that
//! repeats or meets a normal read of the same session adds nothing twice. When none is left the
//! pass writes `usage_backfilled_at` to `meta`.

use super::{store, UsageMsg};
use crate::agents::{Agents, Transcript};
use crate::archive;
use crate::db::Db;
use crate::model::{Session, Stamp};
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::{params, TransactionBehavior};
use std::path::PathBuf;

/// Sessions the daemon counts per tick, so the board and the picker stay quick.
pub const BATCH: usize = 5;
/// A pass stops early after this long, so a few very long transcripts do not hold up a tick.
const PASS_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);
/// `meta` key set when no session is waiting any more.
pub const DONE_KEY: &str = "usage_backfilled_at";

/// What a pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    pub counted: usize,
    pub unreadable: usize,
    /// Sessions still waiting for the next pass.
    pub left: usize,
}

/// The sessions to count: those of an enabled agent that have a transcript or a copy of one and no
/// usage yet (a session of an agent that is switched off waits until it is switched on).
const WAITING: &str = "FROM session s WHERE transcript_state IN ('present','archived')
    AND agent IN (SELECT value FROM json_each(?1))
    AND NOT EXISTS (SELECT 1 FROM usage_state w WHERE w.session_id=s.id)";

fn enabled_ids(agents: &Agents) -> String {
    serde_json::json!(agents.iter().map(|a| a.id()).collect::<Vec<_>>()).to_string()
}

/// Count up to `limit` sessions that have no usage yet, the most recently active first.
pub fn run_batch(db: &mut Db, paths: &Paths, agents: &Agents, limit: usize) -> Result<Pass> {
    let enabled = enabled_ids(agents);
    let ids: Vec<String> = db
        .conn
        .prepare(&format!(
            "SELECT id {WAITING} ORDER BY last_activity_at DESC LIMIT ?2"
        ))?
        .query_map(params![enabled, limit as i64], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut pass = Pass::default();
    let started = std::time::Instant::now();
    for id in &ids {
        if started.elapsed() > PASS_BUDGET {
            break;
        }
        let Some(s) = db.session_by_id(id)? else {
            continue;
        };
        // The files are read while the write lock is held: a scan in another process that appends
        // between this read and the store would otherwise be counted twice.
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !store::is_counted(&tx, id)? {
            match read(&s, paths, agents) {
                Ok(msgs) => {
                    store::replace(&tx, id, &msgs)?;
                    pass.counted += 1;
                }
                Err(_) => {
                    store::mark_unreadable(&tx, id)?;
                    pass.unreadable += 1;
                }
            }
        }
        tx.commit()?;
    }
    pass.left = waiting(db, agents)?;
    if pass.left == 0 && db.meta_get(DONE_KEY)?.is_none() {
        db.meta_set(DONE_KEY, &crate::util::now_ms().to_string())?;
    }
    Ok(pass)
}

/// Count every session that has no usage yet, in batches of `BATCH`.
pub fn run_all(db: &mut Db, paths: &Paths, agents: &Agents) -> Result<Pass> {
    let mut total = Pass::default();
    loop {
        let pass = run_batch(db, paths, agents, BATCH)?;
        total.counted += pass.counted;
        total.unreadable += pass.unreadable;
        total.left = pass.left;
        if pass.counted + pass.unreadable == 0 || pass.left == 0 {
            return Ok(total);
        }
    }
}

/// How many sessions have no usage yet.
pub fn waiting(db: &Db, agents: &Agents) -> Result<usize> {
    let n: i64 = db.conn.query_row(
        &format!("SELECT count(*) {WAITING}"),
        [enabled_ids(agents)],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// The messages of a session: its transcript (or eye's copy of it when that is gone) and, while
/// the original is there, its sub-agents' transcripts.
fn read(s: &Session, paths: &Paths, agents: &Agents) -> Result<Vec<UsageMsg>> {
    let agent = agents
        .of_session(s)
        .context("the agent of the session is not enabled")?;
    let readable = archive::readable(s, paths, agent.as_ref())?;
    let mut msgs = agent.usage_msgs(&readable.path)?;
    if let Some(path) = s
        .transcript_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.exists())
    {
        let t = Transcript {
            project_dir: s.project_dir.clone().unwrap_or_default(),
            session_id: s.id.clone(),
            stamp: Stamp::of_file(&path).unwrap_or(Stamp { size: 0, mtime: 0 }),
            path,
        };
        for src in agent.subagents(&t).transcripts {
            // A sub-agent that cannot be read is left out, as the scan leaves it out of `tok_sub`.
            if let Ok(m) = agent.usage_msgs(&src.path) {
                msgs.extend(m);
            }
        }
    }
    Ok(msgs)
}
