//! The stored turn events of a session (`turn`) and whether it has been counted (`turn_state`). A
//! session with a state row has its turns; one without has not, and `backfill` will do it.

use super::TurnEvent;
use anyhow::Result;
use rusqlite::{params, Connection};

/// Has the session been counted, or given up on?
pub fn is_counted(conn: &Connection, sid: &str) -> Result<bool> {
    Ok(conn
        .prepare("SELECT 1 FROM turn_state WHERE session_id=?1")?
        .exists([sid])?)
}

/// Store the events of one read. `replaces`: they stand for the whole session, so what is stored is
/// replaced. Otherwise they are what is new and are added, but only to a session that has been
/// counted (and is readable): one that has not is left to the backfill, which reads the whole transcript, so an event
/// is never stored by both.
pub fn record(tx: &Connection, sid: &str, turns: &[TurnEvent], replaces: bool) -> Result<()> {
    if replaces {
        tx.execute("DELETE FROM turn WHERE session_id=?1", [sid])?;
    } else if !tx
        .prepare("SELECT 1 FROM turn_state WHERE session_id=?1 AND unreadable=0")?
        .exists([sid])?
    {
        return Ok(());
    }
    insert(tx, sid, turns)?;
    tx.execute(
        "INSERT INTO turn_state(session_id, unreadable) VALUES(?1,0)
         ON CONFLICT(session_id) DO UPDATE SET unreadable=0",
        [sid],
    )?;
    Ok(())
}

fn insert(tx: &Connection, sid: &str, turns: &[TurnEvent]) -> Result<()> {
    let mut stmt =
        tx.prepare_cached("INSERT INTO turn(session_id, ts, kind, n) VALUES(?1,?2,?3,?4)")?;
    for t in turns {
        stmt.execute(params![sid, t.ts, t.kind.as_str(), t.n])?;
    }
    Ok(())
}

/// Note that a session cannot be read, so the backfill does not try it again at every pass. A full
/// scan reads it if it comes back.
pub fn mark_unreadable(tx: &Connection, sid: &str) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO turn_state(session_id, unreadable) VALUES(?1,1)",
        [sid],
    )?;
    Ok(())
}
