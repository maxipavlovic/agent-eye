//! Sessions read before eye kept a work timeline have their cursors at the end of their files, so
//! the normal read never sees their past messages. The backfill reads only the timestamps of those
//! sessions, newest first, a few per pass, and touches nothing else (no cursor, output, token
//! count or summary). A session is counted once: the daemon does a few per tick, `eye scan` all of
//! them, and `store::record` leaves a counted session alone, so a pass that repeats or meets a
//! normal read of the same session adds nothing twice.

use super::store;
use crate::agents::Agents;
use crate::archive;
use crate::db::Db;
use crate::model::Session;
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::{params, TransactionBehavior};

/// Sessions the daemon counts per tick, so the board and the picker stay quick.
pub const BATCH: usize = 5;

/// What a pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    pub counted: usize,
    pub unreadable: usize,
    /// Sessions still waiting for the next pass.
    pub left: usize,
}

/// The sessions to count: those of an enabled agent that have a transcript or a copy of one and no
/// timeline yet (a session of an agent that is switched off waits until it is switched on).
const WAITING: &str = "FROM session s WHERE transcript_state IN ('present','archived')
    AND agent IN (SELECT value FROM json_each(?1))
    AND NOT EXISTS (SELECT 1 FROM worked_state w WHERE w.session_id=s.id)";

fn enabled_ids(agents: &Agents) -> String {
    serde_json::json!(agents.iter().map(|a| a.id()).collect::<Vec<_>>()).to_string()
}

/// Count up to `limit` sessions that have no timeline yet, the most recently active first.
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
    for id in &ids {
        let Some(s) = db.session_by_id(id)? else {
            continue;
        };
        let events = read(&s, paths, agents);
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // A normal read may have counted the session since the query above.
        if !store::is_counted(&tx, id)? {
            match events {
                Ok(events) => {
                    store::record(&tx, id, &events, true)?;
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
    Ok(pass)
}

/// Count every session that has no timeline yet, in batches of `BATCH`.
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

/// How many sessions have no timeline yet.
pub fn waiting(db: &Db, agents: &Agents) -> Result<usize> {
    let n: i64 = db.conn.query_row(
        &format!("SELECT count(*) {WAITING}"),
        [enabled_ids(agents)],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// The messages of a session, from its transcript or, when that is gone, eye's copy of it.
fn read(s: &Session, paths: &Paths, agents: &Agents) -> Result<Vec<crate::worked::WorkEvent>> {
    let agent = agents
        .of_session(s)
        .context("the agent of the session is not enabled")?;
    let readable = archive::readable(s, paths, agent.as_ref())?;
    agent.work_events(&readable.path)
}
