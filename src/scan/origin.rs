//! Sessions indexed before eye kept where they were started have their cursors at the end of their
//! files, so the normal read never sees the first lines again. This pass reads only the first lines
//! of those transcripts (`Agent::origin_raw`) and touches nothing else: no cursor, count, title or
//! summary. A session is looked at once (`origin_raw` is '' when nothing was found); the sessions of
//! an agent that is switched off wait until it is switched on.

use crate::agents::{origin, Agents};
use crate::db::Db;
use anyhow::Result;
use rusqlite::{params, TransactionBehavior};
use std::path::Path;

/// `(id, agent, cwd, transcript path, transcript state)` of a session to look at.
type Waiting = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Look at every session that has not been looked at. Returns how many were.
pub fn backfill(db: &mut Db, agents: &Agents, night: Option<&Path>) -> Result<usize> {
    let waiting: Vec<Waiting> = db
        .conn
        .prepare(
            "SELECT id, agent, cwd, transcript_path, transcript_state FROM session
             WHERE origin_raw IS NULL",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if waiting.is_empty() {
        return Ok(0);
    }
    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut n = 0;
    for (id, agent_id, cwd, path, state) in waiting {
        let agent_id = if agent_id.is_empty() {
            "claude"
        } else {
            &agent_id
        };
        let Some(agent) = agents.iter().find(|a| a.id() == agent_id) else {
            continue;
        };
        let raw = match (&path, state.as_deref()) {
            (Some(p), Some("present")) if Path::new(p).exists() => agent.origin_raw(Path::new(p)),
            _ => None,
        };
        let mapped = origin::of_session(agent_id, raw.as_deref(), cwd.as_deref(), night);
        tx.execute(
            "UPDATE session SET origin=?2, origin_raw=?3 WHERE id=?1 AND origin_raw IS NULL",
            params![id, mapped, raw.unwrap_or_default()],
        )?;
        n += 1;
    }
    tx.commit()?;
    Ok(n)
}
