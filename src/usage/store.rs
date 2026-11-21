//! The stored usage of a session (`usage_day`) and where its count stands (`usage_state`). A
//! session with a state row has been counted; one without has not, and `backfill` will do it. A read
//! that adds to a session that was never counted would be counted twice (once here, once by the
//! backfill, which reads the whole transcript), so such a read is left out.

use super::{by_day, UsageMsg};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

fn state(conn: &Connection, sid: &str) -> Result<Option<bool>> {
    Ok(conn
        .query_row(
            "SELECT unreadable FROM usage_state WHERE session_id=?1",
            [sid],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .map(|u| u != 0))
}

/// Has the session been counted, or given up on?
pub fn is_counted(conn: &Connection, sid: &str) -> Result<bool> {
    Ok(state(conn, sid)?.is_some())
}

fn insert(tx: &Connection, sid: &str, msgs: &[UsageMsg]) -> Result<()> {
    for ((day, model), t) in by_day(msgs, &crate::worked::summary::local_offset_ms) {
        tx.execute(
            "INSERT INTO usage_day(session_id, day, model, tok_in, tok_out, tok_cw, tok_cr) VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(session_id, day, model) DO UPDATE SET tok_in=tok_in+excluded.tok_in,
               tok_out=tok_out+excluded.tok_out, tok_cw=tok_cw+excluded.tok_cw, tok_cr=tok_cr+excluded.tok_cr",
            params![sid, day, model, t[0], t[1], t[2], t[3]],
        )?;
    }
    Ok(())
}

/// Add what is new to a session that has been counted. Anything else is left to the backfill.
pub fn add(tx: &Connection, sid: &str, msgs: &[UsageMsg]) -> Result<()> {
    if msgs.is_empty() || state(tx, sid)? != Some(false) {
        return Ok(());
    }
    insert(tx, sid, msgs)
}

/// Store the whole usage of a session, replacing what was stored.
pub fn replace(tx: &Connection, sid: &str, msgs: &[UsageMsg]) -> Result<()> {
    tx.execute("DELETE FROM usage_day WHERE session_id=?1", [sid])?;
    insert(tx, sid, msgs)?;
    tx.execute(
        "INSERT INTO usage_state(session_id, unreadable) VALUES(?1,0)
         ON CONFLICT(session_id) DO UPDATE SET unreadable=0",
        [sid],
    )?;
    Ok(())
}

/// Forget a session's usage: the transcript was rewritten, or a sub-agent's was, so what is stored
/// no longer adds up. The backfill counts it again from the files.
pub fn invalidate(tx: &Connection, sid: &str) -> Result<()> {
    tx.execute("DELETE FROM usage_day WHERE session_id=?1", [sid])?;
    tx.execute("DELETE FROM usage_state WHERE session_id=?1", [sid])?;
    Ok(())
}

/// Note that a session cannot be read, so the backfill does not try it again at every pass. A
/// rewrite of the transcript (`invalidate`) gives it another chance.
pub fn mark_unreadable(tx: &Connection, sid: &str) -> Result<()> {
    tx.execute("DELETE FROM usage_day WHERE session_id=?1", [sid])?;
    tx.execute(
        "INSERT INTO usage_state(session_id, unreadable) VALUES(?1,1)
         ON CONFLICT(session_id) DO UPDATE SET unreadable=1",
        [sid],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const D0: i64 = 20_000 * 86_400_000;

    fn msg(ts: i64, model: &str, i: i64) -> UsageMsg {
        UsageMsg {
            ts,
            model: model.into(),
            input: i,
            output: 1,
            ..Default::default()
        }
    }

    fn total(db: &Db, sid: &str) -> (i64, i64) {
        db.conn
            .query_row(
                "SELECT COALESCE(sum(tok_in),0), count(*) FROM usage_day WHERE session_id=?1",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn a_session_that_was_not_counted_ignores_an_incremental_read() {
        let db = Db::open_memory().unwrap();
        add(&db.conn, "s", &[msg(D0, "m", 5)]).unwrap();
        assert!(!is_counted(&db.conn, "s").unwrap());
        assert_eq!(total(&db, "s"), (0, 0));
    }

    #[test]
    fn a_replacing_store_counts_the_session_and_a_later_read_adds_to_it() {
        let db = Db::open_memory().unwrap();
        replace(&db.conn, "s", &[msg(D0, "m", 5), msg(D0 + 1, "n", 2)]).unwrap();
        add(&db.conn, "s", &[msg(D0 + 2, "m", 3)]).unwrap();
        assert_eq!(total(&db, "s"), (10, 2));
        replace(&db.conn, "s", &[msg(D0, "m", 1)]).unwrap();
        assert_eq!(total(&db, "s"), (1, 1));
    }

    #[test]
    fn an_unreadable_session_ignores_incremental_reads_until_invalidated() {
        let db = Db::open_memory().unwrap();
        mark_unreadable(&db.conn, "s").unwrap();
        assert!(is_counted(&db.conn, "s").unwrap());
        add(&db.conn, "s", &[msg(D0, "m", 5)]).unwrap();
        assert_eq!(total(&db, "s"), (0, 0));
        invalidate(&db.conn, "s").unwrap();
        assert!(!is_counted(&db.conn, "s").unwrap());
    }

    #[test]
    fn invalidating_drops_the_rows_and_the_state() {
        let db = Db::open_memory().unwrap();
        replace(&db.conn, "s", &[msg(D0, "m", 5)]).unwrap();
        invalidate(&db.conn, "s").unwrap();
        assert_eq!(total(&db, "s"), (0, 0));
        assert!(!is_counted(&db.conn, "s").unwrap());
    }
}
