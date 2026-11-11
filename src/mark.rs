//! `eye mark`: put a session in a column by hand, and find "the session I am in".

use crate::agents::Agents;
use crate::db::Db;
use crate::model::Column;
use crate::util::now_ms;
use anyhow::{Context, Result};
use rusqlite::params;
use std::path::Path;

/// Set the manual column (`auto` clears it, `archive` archives). `next` replaces the next step.
pub fn mark(db: &Db, id: &str, status: &str, next: Option<&str>) -> Result<String> {
    let now = now_ms();
    let want = status.trim().to_lowercase();
    if want == "auto" {
        db.conn.execute(
            "UPDATE session SET column_manual=NULL, column_manual_at=NULL, archived=0 WHERE id=?1",
            [id],
        )?;
    } else {
        let col = Column::parse(&want).with_context(|| {
            format!("unknown status '{status}' (done, paused, waiting, needs-you, archive, auto)")
        })?;
        match col {
            Column::Ideas | Column::Running => {
                anyhow::bail!("a session can be marked done, paused, waiting, needs-you or archive")
            }
            Column::Archive => {
                db.conn.execute(
                    "UPDATE session SET archived=1, column_manual=NULL, column_manual_at=NULL WHERE id=?1",
                    [id],
                )?;
            }
            c => {
                db.conn.execute(
                    "UPDATE session SET archived=0, column_manual=?2, column_manual_at=?3 WHERE id=?1",
                    params![id, c.as_str(), now],
                )?;
            }
        }
    }
    // putting it somewhere by hand is acting on a reminder that came due
    let finished = matches!(Column::parse(&want), Some(Column::Done | Column::Archive));
    crate::remind::answered(db, id, now, finished)?;
    if let Some(n) = next.map(str::trim).filter(|n| !n.is_empty()) {
        db.conn.execute(
            "UPDATE session SET next_step=?2 WHERE id=?1",
            params![id, n],
        )?;
    }
    Ok(want)
}

fn same_dir(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// The session the caller is in: a live session of an enabled agent with this cwd (busy first,
/// interactive before headless runs, newest first), else the newest indexed transcript for the cwd.
pub fn current_session(db: &Db, agents: &Agents, cwd: &Path) -> Result<Option<String>> {
    let cwd = cwd.to_string_lossy();
    let mut live: Vec<_> = agents
        .iter()
        .flat_map(|a| a.live_sessions())
        .filter(|s| s.alive && s.cwd.as_deref().is_some_and(|c| same_dir(c, &cwd)))
        .collect();
    live.sort_by_key(|s| {
        (
            s.status.as_deref() != Some("busy"),
            s.entrypoint.as_deref() == Some("sdk-cli"),
            std::cmp::Reverse(s.updated_at.unwrap_or(0)),
        )
    });
    for s in &live {
        if db.session_by_id(&s.session_id)?.is_some() {
            return Ok(Some(s.session_id.clone()));
        }
    }
    let mut best: Option<(i64, String)> = None;
    for s in db.all_sessions()? {
        if s.transcript_state.as_deref() == Some("present")
            && s.cwd.as_deref().is_some_and(|c| same_dir(c, &cwd))
        {
            let t = s.last_activity_at.unwrap_or(0);
            if best.as_ref().is_none_or(|(b, _)| t > *b) {
                best = Some((t, s.id));
            }
        }
    }
    Ok(best.map(|(_, id)| id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(db: &Db, id: &str, cwd: &str, at: i64) {
        db.conn
            .execute(
                "INSERT INTO session(id,cwd,last_activity_at,transcript_state,kind) VALUES(?1,?2,?3,'present','interactive')",
                params![id, cwd, at],
            )
            .unwrap();
    }

    #[test]
    fn marks_and_clears() {
        let db = Db::open_memory().unwrap();
        add(&db, "a", "/x", 1);
        mark(&db, "a", "needs-you", Some("reply to Dana")).unwrap();
        let s = db.session_by_id("a").unwrap().unwrap();
        assert_eq!(s.column_manual.as_deref(), Some("needs_you"));
        assert_eq!(s.next_step.as_deref(), Some("reply to Dana"));
        mark(&db, "a", "archive", None).unwrap();
        assert!(db.session_by_id("a").unwrap().unwrap().archived);
        mark(&db, "a", "auto", None).unwrap();
        let s = db.session_by_id("a").unwrap().unwrap();
        assert!(!s.archived && s.column_manual.is_none());
        assert!(mark(&db, "a", "running", None).is_err());
        assert!(mark(&db, "a", "idea", None).is_err());
        assert!(mark(&db, "a", "bogus", None).is_err());
    }

    #[test]
    fn current_prefers_live_busy_then_newest_transcript() {
        let tmp = tempfile::tempdir().unwrap();
        let sessions_dir = tmp.path().join("c/sessions");
        let agents = Agents::from_vec(vec![std::sync::Arc::new(
            crate::agents::claude::Claude::new(tmp.path().join("c")),
        )]);
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let db = Db::open_memory().unwrap();
        add(&db, "old", "/work/p", 10);
        add(&db, "new", "/work/p", 20);
        add(&db, "live", "/work/p", 5);
        assert_eq!(
            current_session(&db, &agents, Path::new("/work/p"))
                .unwrap()
                .as_deref(),
            Some("new")
        );
        let pid = std::process::id();
        std::fs::write(
            sessions_dir.join(format!("{pid}.json")),
            format!(r#"{{"pid":{pid},"sessionId":"live","cwd":"/work/p","status":"busy","entrypoint":"cli","updatedAt":9}}"#),
        )
        .unwrap();
        assert_eq!(
            current_session(&db, &agents, Path::new("/work/p"))
                .unwrap()
                .as_deref(),
            Some("live")
        );
        assert_eq!(
            current_session(&db, &agents, Path::new("/other")).unwrap(),
            None
        );
    }
}
