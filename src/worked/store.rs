//! The stored hours of a session (`worked_hour`) and where its count stands (`worked_state`).
//! A session with a state row has been counted; one without has not, and `backfill` will do it.

use super::{bucket, Hour, WorkEvent};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// Where the count of a session stands.
struct State {
    /// The time of the last message counted.
    last_ts: Option<i64>,
    /// The backfill found nothing to read (no transcript, no archive copy, or it did not parse).
    unreadable: bool,
}

fn state(conn: &Connection, sid: &str) -> Result<Option<State>> {
    Ok(conn
        .query_row(
            "SELECT last_ts, unreadable FROM worked_state WHERE session_id=?1",
            [sid],
            |r| {
                Ok(State {
                    last_ts: r.get(0)?,
                    unreadable: r.get::<_, i64>(1)? != 0,
                })
            },
        )
        .optional()?)
}

/// Has the session been counted, or given up on?
pub fn is_counted(conn: &Connection, sid: &str) -> Result<bool> {
    Ok(state(conn, sid)?.is_some())
}

/// Store the messages of one read. `replaces`: they stand for the whole session, so what is stored
/// is replaced. Otherwise they are what is new, and they are added to what is stored, but only
/// for a session that has been counted: one that has not is left to the backfill, which reads the
/// whole transcript, so a message is never counted by both. Messages at or before the last one
/// counted are skipped, which makes a repeated read harmless.
pub fn record(tx: &Connection, sid: &str, events: &[WorkEvent], replaces: bool) -> Result<()> {
    let last = if replaces {
        tx.execute("DELETE FROM worked_hour WHERE session_id=?1", [sid])?;
        None
    } else {
        match state(tx, sid)? {
            Some(State {
                last_ts,
                unreadable: false,
            }) => last_ts,
            _ => return Ok(()),
        }
    };
    let (hours, new_last) = bucket(last, events);
    for (hour, h) in &hours {
        tx.execute(
            "INSERT INTO worked_hour(session_id, hour, prompts, events, active_ms) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(session_id, hour) DO UPDATE SET prompts=prompts+excluded.prompts,
               events=events+excluded.events, active_ms=active_ms+excluded.active_ms",
            params![sid, hour, h.prompts, h.events, h.active_ms],
        )?;
    }
    tx.execute(
        "INSERT INTO worked_state(session_id, last_ts, unreadable) VALUES(?1,?2,0)
         ON CONFLICT(session_id) DO UPDATE SET last_ts=excluded.last_ts, unreadable=0",
        params![sid, new_last],
    )?;
    Ok(())
}

/// Note that a session cannot be read, so the backfill does not try it again at every pass. A
/// full scan reads it if it comes back.
pub fn mark_unreadable(tx: &Connection, sid: &str) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO worked_state(session_id, last_ts, unreadable) VALUES(?1,NULL,1)",
        [sid],
    )?;
    Ok(())
}

/// The stored hours of a session, oldest first, with the UTC hour of each.
pub type Hours = Vec<(i64, Hour)>;

/// The hours of a session with the time of its last message; None while it has not been counted
/// or has no messages.
pub fn load(conn: &Connection, sid: &str) -> Result<Option<(Hours, i64)>> {
    let Some(State {
        last_ts: Some(last),
        unreadable: false,
    }) = state(conn, sid)?
    else {
        return Ok(None);
    };
    let mut st = conn.prepare(
        "SELECT hour, prompts, events, active_ms FROM worked_hour WHERE session_id=?1 ORDER BY hour",
    )?;
    let hours: Hours = st
        .query_map([sid], |r| {
            Ok((
                r.get(0)?,
                Hour {
                    prompts: r.get(1)?,
                    events: r.get(2)?,
                    active_ms: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok((!hours.is_empty()).then_some((hours, last)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const MIN: i64 = 60_000;
    const T0: i64 = 1_000 * 3_600_000;

    fn ev(min: i64, prompt: bool) -> WorkEvent {
        WorkEvent {
            ts: T0 + min * MIN,
            prompt,
        }
    }

    fn active(db: &Db, sid: &str) -> i64 {
        load(&db.conn, sid)
            .unwrap()
            .map_or(0, |(h, _)| h.iter().map(|(_, h)| h.active_ms).sum())
    }

    #[test]
    fn a_session_that_was_not_counted_ignores_an_incremental_read() {
        let db = Db::open_memory().unwrap();
        record(&db.conn, "s", &[ev(0, true), ev(5, false)], false).unwrap();
        assert!(!is_counted(&db.conn, "s").unwrap());
        assert!(load(&db.conn, "s").unwrap().is_none());
    }

    #[test]
    fn a_replacing_read_stores_the_session_and_a_later_one_adds_to_it() {
        let db = Db::open_memory().unwrap();
        record(&db.conn, "s", &[ev(0, true), ev(5, false)], true).unwrap();
        record(&db.conn, "s", &[ev(8, false), ev(12, true)], false).unwrap();
        assert_eq!(active(&db, "s"), 12 * MIN);
        let (hours, last) = load(&db.conn, "s").unwrap().unwrap();
        assert_eq!(hours[0].1.prompts, 2);
        assert_eq!(last, ev(12, true).ts);
    }

    #[test]
    fn a_replacing_read_drops_what_was_stored_before() {
        let db = Db::open_memory().unwrap();
        record(&db.conn, "s", &[ev(0, true), ev(5, false)], true).unwrap();
        record(&db.conn, "s", &[ev(0, true), ev(3, false)], true).unwrap();
        assert_eq!(active(&db, "s"), 3 * MIN);
    }

    #[test]
    fn reading_the_same_messages_twice_counts_them_once() {
        let db = Db::open_memory().unwrap();
        let all = [ev(0, true), ev(5, false)];
        record(&db.conn, "s", &all, true).unwrap();
        record(&db.conn, "s", &all, false).unwrap();
        record(&db.conn, "s", &all, false).unwrap();
        assert_eq!(active(&db, "s"), 5 * MIN);
        assert_eq!(load(&db.conn, "s").unwrap().unwrap().0[0].1.events, 2);
    }

    #[test]
    fn an_unreadable_session_has_no_hours_and_ignores_incremental_reads() {
        let db = Db::open_memory().unwrap();
        mark_unreadable(&db.conn, "s").unwrap();
        assert!(is_counted(&db.conn, "s").unwrap());
        record(&db.conn, "s", &[ev(0, true), ev(5, false)], false).unwrap();
        assert!(load(&db.conn, "s").unwrap().is_none());
        record(&db.conn, "s", &[ev(0, true), ev(5, false)], true).unwrap();
        assert_eq!(active(&db, "s"), 5 * MIN);
    }

    #[test]
    fn a_session_without_messages_has_nothing_to_show() {
        let db = Db::open_memory().unwrap();
        record(&db.conn, "s", &[], true).unwrap();
        assert!(is_counted(&db.conn, "s").unwrap());
        assert!(load(&db.conn, "s").unwrap().is_none());
    }
}
