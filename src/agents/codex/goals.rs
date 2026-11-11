//! Codex's `/goal`: a thread with an `active` goal starts its own turns whenever it is idle, which
//! makes it a loop. The goals live in `goals_1.sqlite` next to the rollouts. The file is Codex's, so
//! it is only ever opened read-only, and a missing file, table or row is simply no goal.

use crate::activity::one_line;
use crate::agents::LoopInfo;
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::Path;

const FILE: &str = "goals_1.sqlite";
/// How much of the objective the card line shows.
const OBJECTIVE_MAX: usize = 60;

/// `file:` URI of a path, with the characters that would end it early written as `%XX`.
fn uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out.push_str("?immutable=1");
    out
}

fn query(conn: &Connection) -> rusqlite::Result<Vec<(String, String, i64)>> {
    let mut st = conn.prepare(
        "SELECT thread_id, objective, COALESCE(created_at_ms, 0) FROM thread_goals WHERE status = 'active'",
    )?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    rows.collect()
}

/// Read the active goals: a plain read-only open when the write-ahead log is there to be read
/// (Codex is running), which sees what has not been checkpointed; otherwise, or when that fails, an
/// immutable open, which never touches a file next to the database.
fn read(path: &Path) -> Option<Vec<(String, String, i64)>> {
    let wal = |suffix: &str| {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        Path::new(&p).exists()
    };
    if wal("-wal") && wal("-shm") {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        if let Some(rows) = Connection::open_with_flags(path, flags)
            .ok()
            .and_then(|c| query(&c).ok())
        {
            return Some(rows);
        }
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(uri(path), flags).ok()?;
    query(&conn).ok()
}

/// The threads with an active goal, by lower-case thread id.
pub fn active(dir: &Path) -> HashMap<String, LoopInfo> {
    let path = dir.join(FILE);
    if !path.is_file() {
        return HashMap::new();
    }
    read(&path)
        .unwrap_or_default()
        .into_iter()
        .map(|(thread, objective, created)| {
            let objective = one_line(&objective);
            let shown: String = objective.chars().take(OBJECTIVE_MAX).collect();
            let shown = if shown.len() < objective.len() {
                format!("{shown}…")
            } else {
                shown
            };
            (
                thread.to_ascii_lowercase(),
                LoopInfo {
                    kind: "goal".into(),
                    schedule: format!("goal: {shown}"),
                    next_at: None,
                    prompt: objective,
                    started_at: created,
                    cron: None,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dir: &Path, rows: &[(&str, &str, &str)]) {
        let conn = Connection::open(dir.join(FILE)).unwrap();
        conn.execute_batch(
            "CREATE TABLE thread_goals(thread_id TEXT PRIMARY KEY, goal_id TEXT, objective TEXT, status TEXT,
               token_budget INTEGER, tokens_used INTEGER, time_used_seconds INTEGER, created_at_ms INTEGER, updated_at_ms INTEGER);",
        )
        .unwrap();
        for (thread, objective, status) in rows {
            conn.execute(
                "INSERT INTO thread_goals(thread_id, goal_id, objective, status, created_at_ms) VALUES(?1, 'g', ?2, ?3, 5)",
                [thread, objective, status],
            )
            .unwrap();
        }
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn only_an_active_goal_is_a_loop() {
        let d = tempfile::tempdir().unwrap();
        make(
            d.path(),
            &[
                ("0AAA", "ship the\nrelease", "active"),
                ("0bbb", "old", "complete"),
                ("0ccc", "later", "paused"),
            ],
        );
        let goals = active(d.path());
        assert_eq!(goals.len(), 1);
        let g = &goals["0aaa"];
        assert_eq!(
            (g.kind.as_str(), g.schedule.as_str()),
            ("goal", "goal: ship the release")
        );
        assert_eq!((g.prompt.as_str(), g.started_at), ("ship the release", 5));
    }

    #[test]
    fn a_long_objective_is_cut_for_the_line_and_kept_whole_for_the_panel() {
        let d = tempfile::tempdir().unwrap();
        let long = "word ".repeat(40);
        make(d.path(), &[("t", &long, "active")]);
        let g = &active(d.path())["t"];
        assert!(g.schedule.ends_with('…') && g.schedule.chars().count() <= 6 + OBJECTIVE_MAX + 1);
        assert!(g.prompt.chars().count() > OBJECTIVE_MAX);
    }

    #[test]
    fn a_missing_file_or_table_is_no_goal_and_nothing_is_created() {
        let d = tempfile::tempdir().unwrap();
        assert!(active(d.path()).is_empty());
        Connection::open(d.path().join(FILE))
            .unwrap()
            .execute_batch("CREATE TABLE other(x)")
            .unwrap();
        let before = listing(d.path());
        assert!(active(d.path()).is_empty());
        std::fs::write(d.path().join(FILE), "not a database").unwrap();
        assert!(active(d.path()).is_empty());
        assert_eq!(listing(d.path()), before);
    }

    #[test]
    fn a_database_in_write_ahead_mode_is_read_without_new_files() {
        let d = tempfile::tempdir().unwrap();
        make(d.path(), &[("t", "x", "active")]);
        let conn = Connection::open(d.path().join(FILE)).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute("UPDATE thread_goals SET objective='y'", [])
            .unwrap();
        // the connection is still open, so the log and its index file are there to read
        let before = listing(d.path());
        assert_eq!(active(d.path())["t"].prompt, "y");
        assert_eq!(listing(d.path()), before);
        drop(conn);
        assert_eq!(active(d.path())["t"].prompt, "y");
    }

    #[test]
    fn a_path_with_odd_characters_still_opens_immutably() {
        let d = tempfile::tempdir().unwrap();
        let odd = d.path().join("a b#c?d%e");
        std::fs::create_dir(&odd).unwrap();
        make(&odd, &[("t", "x", "active")]);
        assert_eq!(active(&odd).len(), 1);
    }
}
