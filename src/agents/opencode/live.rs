//! Which OpenCode sessions are working. OpenCode keeps its sessions in a background service, so
//! no process or lock says "this session is open". What it does write, on the session's row, is
//! `time_busy` when a turn starts and `time_idle` when it ends. A turn is open while the last of
//! them is `time_busy`. A session whose turn ended a few minutes ago is still shown as idle (it
//! is most likely open in a terminal), then it drops out.

use super::store;
use crate::agents::LiveSession;
use crate::util::now_ms;
use std::path::Path;

/// How long after a turn ended a session still counts as open (idle).
pub const IDLE_GRACE_MS: i64 = 10 * 60 * 1000;
/// A turn that began longer ago than this and never ended is a crashed one, not a long turn.
pub const BUSY_MAX_MS: i64 = 6 * 60 * 60 * 1000;

/// The sessions of every database in `dir` that are busy, or idle for less than the grace time.
pub fn sessions(dir: &Path) -> Vec<LiveSession> {
    sessions_at(dir, now_ms())
}

pub fn sessions_at(dir: &Path, now: i64) -> Vec<LiveSession> {
    let mut out = vec![];
    for db in store::databases(dir) {
        let Some(c) = store::open(&db) else { continue };
        if !["time_busy", "time_idle", "time_updated"]
            .iter()
            .all(|col| store::has_column(&c, "session_v2", col))
        {
            continue;
        }
        let archived = if store::has_column(&c, "session_v2", "time_archived") {
            " AND time_archived IS NULL"
        } else {
            ""
        };
        let q = format!(
            "SELECT id, directory, time_busy, time_idle, time_updated FROM session_v2
             WHERE (parent_id IS NULL OR parent_id = '') AND time_busy IS NOT NULL{archived}"
        );
        let Ok(mut st) = c.prepare(&q) else { continue };
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        });
        for (id, cwd, busy, idle, updated) in rows.into_iter().flatten().flatten() {
            let open = idle.is_none_or(|i| i < busy);
            let status = if open {
                (now - busy < BUSY_MAX_MS).then_some("busy")
            } else {
                idle.filter(|i| now - i < IDLE_GRACE_MS).map(|_| "idle")
            };
            let Some(status) = status else { continue };
            out.push(LiveSession {
                pid: 0,
                session_id: id,
                cwd: cwd.filter(|c| !c.is_empty()),
                status: Some(status.into()),
                updated_at: updated,
                entrypoint: None,
                alive: true,
                waiting_for: None,
                remote_url: None,
                loops: vec![],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn busy_idle_and_gone_follow_the_turn_times() {
        let t = tempfile::tempdir().unwrap();
        let c = Connection::open(t.path().join("opencode.db")).unwrap();
        c.execute_batch(
            "CREATE TABLE session_v2 (id text, directory text, parent_id text, time_updated integer, time_busy integer, time_idle integer, time_archived integer);
             INSERT INTO session_v2 VALUES ('ses_run','/w',NULL,1000,1000,NULL,NULL);
             INSERT INTO session_v2 VALUES ('ses_run2','/w',NULL,1000,2000,1500,NULL);
             INSERT INTO session_v2 VALUES ('ses_idle','/w',NULL,1000,1000,2000,NULL);
             INSERT INTO session_v2 VALUES ('ses_old','/w',NULL,1000,1000,2000,NULL);
             INSERT INTO session_v2 VALUES ('ses_stale','/w',NULL,1000,1000,NULL,NULL);
             INSERT INTO session_v2 VALUES ('ses_arch','/w',NULL,1000,1000,NULL,5);
             INSERT INTO session_v2 VALUES ('ses_kid','/w','ses_run',1000,1000,NULL,NULL);
             INSERT INTO session_v2 VALUES ('ses_never','/w',NULL,1000,NULL,NULL,NULL);",
        )
        .unwrap();
        c.execute(
            "UPDATE session_v2 SET time_busy=?1, time_idle=NULL WHERE id IN ('ses_run','ses_run2')",
            [10_000_000],
        )
        .unwrap();
        c.execute(
            "UPDATE session_v2 SET time_busy=?1, time_idle=?2 WHERE id='ses_idle'",
            [9_000_000, 9_900_000],
        )
        .unwrap();
        c.execute(
            "UPDATE session_v2 SET time_busy=?1, time_idle=?2 WHERE id='ses_old'",
            [100, 200],
        )
        .unwrap();
        c.execute(
            "UPDATE session_v2 SET time_busy=-30000000 WHERE id='ses_stale'",
            [],
        )
        .unwrap();
        drop(c);
        let now = 10_000_000 + 1000;
        let mut got: Vec<(String, String)> = sessions_at(t.path(), now)
            .into_iter()
            .map(|s| (s.session_id, s.status.unwrap()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("ses_idle", "idle"),
                ("ses_run", "busy"),
                ("ses_run2", "busy")
            ]
            .map(|(a, b)| (a.to_string(), b.to_string()))
        );
    }

    #[test]
    fn a_database_of_the_older_layout_has_no_live_status() {
        let t = tempfile::tempdir().unwrap();
        let c = Connection::open(t.path().join("opencode.db")).unwrap();
        c.execute_batch("CREATE TABLE session (id text, directory text);")
            .unwrap();
        assert!(sessions_at(t.path(), 1).is_empty());
    }
}
