//! `eye statusline`: an opt-in status line command for Claude Code. Claude Code hands the status
//! line command a JSON document on stdin; once a Pro or Max subscriber's first API response is in,
//! it carries `rate_limits.five_hour` and `rate_limits.seven_day` (`used_percentage`, `resets_at`
//! in Unix seconds). This command keeps those as the newest Claude limits (`limits:claude` in
//! `meta`) and prints either the output of the command it wraps (`--wrap`) or a short default line.
//!
//! It runs on every status line refresh, so it opens no config, runs no migration and never
//! waits for the database for more than a moment. On any error it prints what the wrapped command
//! printed, or nothing. `eye install` does not turn it on.

use super::{limits, LimitWindow, PlanLimits};
use crate::util::now_ms;
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// The most of the document that is read.
const INPUT_CAP: u64 = 4 << 20;
/// How long the database may make this command wait.
const DB_WAIT: Duration = Duration::from_millis(25);

/// The limits in a status line document, None when it carries none (an API-key user, or before
/// the first response).
pub fn parse_limits(doc: &Value, now: i64) -> Option<PlanLimits> {
    let r = doc.get("rate_limits")?.as_object()?;
    let window = |name: &str, minutes: Option<i64>| -> Option<LimitWindow> {
        let w = r.get(name)?.as_object()?;
        Some(LimitWindow {
            name: name.to_string(),
            used_percent: w.get("used_percentage")?.as_f64()?,
            window_minutes: minutes,
            resets_at: w
                .get("resets_at")
                .and_then(Value::as_f64)
                .map(|s| (s * 1000.0) as i64),
        })
    };
    let windows: Vec<LimitWindow> = [
        window("five_hour", Some(300)),
        window("seven_day", Some(10_080)),
        window("spend_limit", None),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!windows.is_empty()).then_some(PlanLimits {
        seen_at: now,
        plan: None,
        windows,
    })
}

/// `5-hour 23% · weekly 41%`, empty when there are no limits.
pub fn default_line(l: &PlanLimits) -> String {
    l.windows
        .iter()
        .map(|w| format!("{} {:.0}%", limits::label(w), w.used_percent))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Keep the limits in the database at `db` without creating it, migrating it or waiting for it.
pub fn store(db: &Path, l: &PlanLimits) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(DB_WAIT)?;
    limits::record(&conn, "claude", l).map_err(|e| match e.downcast::<rusqlite::Error>() {
        Ok(e) => e,
        Err(_) => rusqlite::Error::InvalidQuery,
    })
}

/// Read the document from `input`, store its limits, and return what to print: the output of
/// `wrap` (run through `sh -c`, with the document on its stdin) or the default line.
pub fn run(db: &Path, wrap: Option<&str>, mut input: impl Read) -> String {
    let mut raw = Vec::new();
    let _ = (&mut input).take(INPUT_CAP).read_to_end(&mut raw);
    // The wrapped command starts first, so it runs while the database is written.
    let child = wrap.and_then(|cmd| {
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()
    });
    let feeder = child.map(|mut c| {
        let stdin = c.stdin.take();
        let doc = raw.clone();
        let t = std::thread::spawn(move || {
            if let Some(mut s) = stdin {
                let _ = s.write_all(&doc);
            }
        });
        (c, t)
    });
    let limits = serde_json::from_slice::<Value>(&raw)
        .ok()
        .and_then(|d| parse_limits(&d, now_ms()));
    if let Some(l) = &limits {
        let _ = store(db, l);
    }
    match feeder {
        Some((c, t)) => {
            let _ = t.join();
            c.wait_with_output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        }
        None if wrap.is_some() => String::new(),
        None => limits.map(|l| default_line(&l)).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use serde_json::json;

    const DOC: &str = r#"{"session_id":"s","model":{"display_name":"X"},
        "rate_limits":{"five_hour":{"used_percentage":23.5,"resets_at":1738425600},
                       "seven_day":{"used_percentage":41.2,"resets_at":1738857600}}}"#;

    fn home() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("eye.db");
        drop(Db::open(&path).unwrap());
        (tmp, path)
    }

    fn stored(path: &Path) -> Option<PlanLimits> {
        let conn = Connection::open(path).unwrap();
        limits::load(&conn, "claude").unwrap()
    }

    #[test]
    fn the_limits_of_the_document_are_stored_and_a_default_line_is_printed() {
        let (_t, db) = home();
        let out = run(&db, None, DOC.as_bytes());
        assert_eq!(out, "5-hour 24% · weekly 41%");
        let l = stored(&db).unwrap();
        assert_eq!(l.windows.len(), 2);
        assert_eq!(l.windows[0].used_percent, 23.5);
        assert_eq!(l.windows[0].resets_at, Some(1_738_425_600_000));
        assert_eq!(l.windows[1].window_minutes, Some(10_080));
    }

    #[test]
    fn the_wrapped_command_gets_the_document_and_its_output_passes_through() {
        let (_t, db) = home();
        let out = run(
            &db,
            Some("cat | head -c 12; printf ' | mine'"),
            DOC.as_bytes(),
        );
        assert_eq!(out, "{\"session_id | mine");
        assert!(stored(&db).is_some());
    }

    #[test]
    fn bad_json_stores_nothing_and_still_runs_the_wrapped_command() {
        let (_t, db) = home();
        assert_eq!(run(&db, None, &b"not json {"[..]), "");
        assert_eq!(run(&db, Some("printf ok"), &b"not json {"[..]), "ok");
        assert!(stored(&db).is_none());
    }

    #[test]
    fn a_document_without_limits_stores_nothing() {
        let (_t, db) = home();
        let doc = json!({"model": {"display_name": "X"}}).to_string();
        assert_eq!(run(&db, None, doc.as_bytes()), "");
        assert!(stored(&db).is_none());
    }

    #[test]
    fn a_missing_database_and_a_failing_command_never_fail_the_line() {
        let tmp = tempfile::tempdir().unwrap();
        let none = tmp.path().join("none.db");
        assert_eq!(run(&none, None, DOC.as_bytes()), "5-hour 24% · weekly 41%");
        assert!(!none.exists());
        assert_eq!(run(&none, Some("exit 3"), DOC.as_bytes()), "");
        assert_eq!(
            run(&none, Some("nonexistent-command-xyz"), DOC.as_bytes()),
            ""
        );
    }

    #[test]
    fn a_locked_database_costs_a_moment_and_nothing_else() {
        let (_t, db) = home();
        let lock = Connection::open(&db).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let t0 = std::time::Instant::now();
        let out = run(&db, None, DOC.as_bytes());
        assert_eq!(out, "5-hour 24% · weekly 41%");
        assert!(
            t0.elapsed() < Duration::from_millis(500),
            "{:?}",
            t0.elapsed()
        );
        lock.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn it_is_fast() {
        let (_t, db) = home();
        run(&db, None, DOC.as_bytes());
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            run(&db, None, DOC.as_bytes());
        }
        let each = t0.elapsed() / 20;
        assert!(each < Duration::from_millis(50), "{each:?} per call");
    }
}
