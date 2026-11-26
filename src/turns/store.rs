//! The stored turn events of a session (`turn`) and whether it has been counted (`turn_state`). A
//! session with a state row has its turns; one without has not, and `backfill` will do it.

use super::{TurnEvent, TurnKind};
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

/// A forked session starts with a copy of its parent's history, so a prompt that another session
/// already has (same time, same length) is left out: it was typed once.
fn insert(tx: &Connection, sid: &str, turns: &[TurnEvent]) -> Result<()> {
    let mut any =
        tx.prepare_cached("INSERT INTO turn(session_id, ts, kind, n) VALUES(?1,?2,?3,?4)")?;
    let mut prompt = tx.prepare_cached(
        "INSERT INTO turn(session_id, ts, kind, n) SELECT ?1,?2,?3,?4
         WHERE NOT EXISTS (SELECT 1 FROM turn WHERE kind='prompt' AND ts=?2 AND n=?4 AND session_id<>?1)",
    )?;
    for t in turns {
        let stmt = if t.kind == TurnKind::Prompt {
            &mut prompt
        } else {
            &mut any
        };
        stmt.execute(params![sid, t.ts, t.kind.as_str(), t.n])?;
    }
    Ok(())
}

/// Drop the turns of sessions that are not in the index. A scan used to store the turns of
/// sessions it left out of the index (excluded and temp folders); `Db::migrate` clears them once.
pub fn drop_unindexed(conn: &Connection) -> Result<()> {
    for table in ["turn", "turn_state"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE session_id NOT IN (SELECT id FROM session)"),
            [],
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::turns::TurnKind::*;

    fn rows(db: &Db, sid: &str) -> i64 {
        db.conn
            .query_row(
                "SELECT count(*) FROM turn WHERE session_id=?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn a_prompt_a_fork_copied_from_its_parent_is_counted_once() {
        let db = Db::open_memory().unwrap();
        let parent = [
            TurnEvent::new(10, Prompt, 20),
            TurnEvent::new(11, Tool, 0),
            TurnEvent::new(20, Prompt, 7),
        ];
        record(&db.conn, "parent", &parent, true).unwrap();
        let fork = [
            TurnEvent::new(10, Prompt, 20),
            TurnEvent::new(11, Tool, 0),
            TurnEvent::new(20, Prompt, 7),
            TurnEvent::new(30, Prompt, 7),
        ];
        record(&db.conn, "fork", &fork, true).unwrap();
        // the fork keeps its own tool call and the prompt it typed itself
        assert_eq!((rows(&db, "parent"), rows(&db, "fork")), (3, 2));
        // reading the parent again does not lose its prompts to the fork
        record(&db.conn, "parent", &parent, true).unwrap();
        assert_eq!(rows(&db, "parent"), 3);
    }

    #[test]
    fn two_prompts_of_one_session_with_the_same_time_and_size_are_both_kept() {
        let db = Db::open_memory().unwrap();
        let twice = [TurnEvent::new(10, Prompt, 5), TurnEvent::new(10, Prompt, 5)];
        record(&db.conn, "a", &twice, true).unwrap();
        assert_eq!(rows(&db, "a"), 2);
    }
}
