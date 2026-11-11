//! The work timeline from the files to the database: the normal read, the backfill of sessions an
//! older eye already read, and the rule that keeps a message from being counted twice.

use agent_eye::agents::Agents;
use agent_eye::archive;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::worked::{backfill, store};
use std::io::Write;
use std::path::{Path, PathBuf};

const SID: &str = "dddddddd-0000-4000-8000-000000000004";
const MIN: i64 = 60_000;

struct Env {
    _tmp: tempfile::TempDir,
    transcript: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

fn line(ts: &str, ty: &str, text: &str) -> String {
    let content = if ty == "user" {
        format!("\"{text}\"")
    } else {
        format!("[{{\"type\":\"text\",\"text\":\"{text}\"}}]")
    };
    format!(
        "{{\"type\":\"{ty}\",\"timestamp\":\"{ts}\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{SID}\",\"message\":{{\"content\":{content}}}}}\n"
    )
}

/// Two days of work: three messages on the 1st (10 min of activity), two on the 3rd (5 min).
fn old_transcript() -> String {
    line("2026-11-01T10:00:00.000Z", "user", "start the thing")
        + &line("2026-11-01T10:05:00.000Z", "assistant", "on it")
        + &line("2026-11-01T10:10:00.000Z", "user", "and the other")
        + &line("2026-11-03T09:00:00.000Z", "user", "back again")
        + &line("2026-11-03T09:05:00.000Z", "assistant", "done")
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        let proj = claude.join("projects/-tmp-demo");
        std::fs::create_dir_all(&proj).unwrap();
        let transcript = proj.join(format!("{SID}.jsonl"));
        std::fs::write(&transcript, old_transcript()).unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        cfg.override_agent_dir("claude", claude);
        let db = Db::open(&paths.db_path()).unwrap();
        Env {
            _tmp: tmp,
            transcript,
            paths,
            cfg,
            db,
        }
    }

    fn agents(&self) -> Agents {
        agent_eye::agents::enabled(&self.cfg)
    }

    fn scan(&mut self) {
        scan(&mut self.db, &self.cfg, false).unwrap();
    }

    /// The database as it was before this feature: sessions read, nothing counted.
    fn forget_hours(&self) {
        self.db.conn.execute("DELETE FROM worked_hour", []).unwrap();
        self.db
            .conn
            .execute("DELETE FROM worked_state", [])
            .unwrap();
    }

    fn backfill(&mut self) -> backfill::Pass {
        let agents = self.agents();
        backfill::run_all(&mut self.db, &self.paths, &agents).unwrap()
    }

    /// (active ms, prompts, events) over the session.
    fn totals(&self) -> (i64, i64, i64) {
        let (hours, _) = store::load(&self.db.conn, SID).unwrap().unwrap();
        hours.iter().fold((0, 0, 0), |a, (_, h)| {
            (a.0 + h.active_ms, a.1 + h.prompts, a.2 + h.events)
        })
    }

    /// Everything the backfill must leave alone: cursors, tokens and the session rows.
    fn untouched(&self) -> String {
        let mut out = String::new();
        let mut st = self
            .db
            .conn
            .prepare("SELECT path, size, mtime, offset, marker FROM cursor ORDER BY path")
            .unwrap();
        for r in st
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}|{}|{:?}\n",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?
                ))
            })
            .unwrap()
        {
            out += &r.unwrap();
        }
        let mut st = self
            .db
            .conn
            .prepare(
                "SELECT id, title, prompt_count, reply_count, last_activity_at, tok_in, tok_out, transcript_state, state_line
                 FROM session ORDER BY id",
            )
            .unwrap();
        for r in st
            .query_map([], |r| {
                Ok(format!(
                    "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}\n",
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?
                ))
            })
            .unwrap()
        {
            out += &r.unwrap();
        }
        out
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

#[test]
fn a_session_read_for_the_first_time_is_counted_by_the_normal_read() {
    let mut e = Env::new();
    e.scan();
    assert_eq!(e.totals(), (15 * MIN, 3, 5));
    assert_eq!(backfill::waiting(&e.db).unwrap(), 0);
}

#[test]
fn a_session_that_grows_is_added_to_and_the_gap_between_reads_counts_once() {
    let mut e = Env::new();
    e.scan();
    append(
        &e.transcript,
        &(line("2026-11-03T09:12:00.000Z", "assistant", "more")
            + &line("2026-11-03T09:14:00.000Z", "user", "thanks")),
    );
    e.scan();
    // 15 minutes before, then 7 more from 09:05 to 09:12 and 2 from 09:12 to 09:14
    assert_eq!(e.totals(), (24 * MIN, 4, 7));
    // a scan that finds nothing new changes nothing
    e.scan();
    assert_eq!(e.totals(), (24 * MIN, 4, 7));
}

#[test]
fn a_full_scan_replaces_the_hours_instead_of_adding() {
    let mut e = Env::new();
    e.scan();
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(e.totals(), (15 * MIN, 3, 5));
}

#[test]
fn an_old_session_is_counted_from_its_timestamps_and_nothing_else_changes() {
    let mut e = Env::new();
    e.scan();
    let counted = e.totals();
    e.forget_hours();
    assert_eq!(backfill::waiting(&e.db).unwrap(), 1);
    let before = e.untouched();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (1, 0, 0));
    assert_eq!(e.totals(), counted);
    assert_eq!(e.untouched(), before);
    // the normal read has nothing new to do afterwards
    e.scan();
    assert_eq!(e.totals(), counted);
}

#[test]
fn running_the_backfill_twice_counts_once() {
    let mut e = Env::new();
    e.scan();
    e.forget_hours();
    e.backfill();
    let once = e.totals();
    let again = e.backfill();
    assert_eq!((again.counted, again.unreadable), (0, 0));
    assert_eq!(e.totals(), once);
}

#[test]
fn a_batch_counts_the_newest_sessions_first_and_says_what_is_left() {
    let mut e = Env::new();
    e.scan();
    e.db.conn
        .execute(
            "INSERT INTO session(id, transcript_state, last_activity_at) VALUES('older','archived',1)",
            [],
        )
        .unwrap();
    e.forget_hours();
    let agents = e.agents();
    let pass = backfill::run_batch(&mut e.db, &e.paths, &agents, 1).unwrap();
    assert_eq!((pass.counted, pass.left), (1, 1));
    assert!(store::load(&e.db.conn, SID).unwrap().is_some());
    let pass = backfill::run_batch(&mut e.db, &e.paths, &agents, 1).unwrap();
    assert_eq!((pass.unreadable, pass.left), (1, 0));
}

#[test]
fn a_read_that_comes_before_the_backfill_is_not_counted_twice() {
    let mut e = Env::new();
    e.scan();
    e.forget_hours();
    // the session grows, and the normal read sees only what is new
    append(
        &e.transcript,
        &line("2026-11-03T09:10:00.000Z", "assistant", "later"),
    );
    e.scan();
    assert!(store::load(&e.db.conn, SID).unwrap().is_none());
    e.backfill();
    assert_eq!(e.totals(), (20 * MIN, 3, 6));
    // and a read after the backfill adds only what came after it
    append(
        &e.transcript,
        &line("2026-11-03T09:12:00.000Z", "user", "one more"),
    );
    e.scan();
    assert_eq!(e.totals(), (22 * MIN, 4, 7));
}

#[test]
fn a_session_whose_transcript_is_gone_is_counted_from_the_archive_copy() {
    let mut e = Env::new();
    e.scan();
    let counted = e.totals();
    let agents = e.agents();
    archive::archive_session(&e.db, &e.paths, &agents, SID).unwrap();
    std::fs::remove_file(&e.transcript).unwrap();
    e.scan();
    let state: String =
        e.db.conn
            .query_row("SELECT transcript_state FROM session", [], |r| r.get(0))
            .unwrap();
    assert_eq!(state, "archived");
    e.forget_hours();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable), (1, 0));
    assert_eq!(e.totals(), counted);
    // nothing was written back into the agent's folder, nor left in eye's temporary one
    assert!(!e.transcript.exists());
    let tmp = e.paths.home.join("tmp");
    assert!(!tmp.exists() || std::fs::read_dir(tmp).unwrap().next().is_none());
}

#[test]
fn a_session_with_no_transcript_and_no_copy_is_given_up_once() {
    let mut e = Env::new();
    e.scan();
    e.forget_hours();
    std::fs::remove_file(&e.transcript).unwrap();
    e.db.conn
        .execute("UPDATE session SET transcript_state='archived'", [])
        .unwrap();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (0, 1, 0));
    assert!(store::load(&e.db.conn, SID).unwrap().is_none());
    assert_eq!(e.backfill().unreadable, 0);
}

#[test]
fn an_empty_database_has_nothing_to_backfill() {
    let mut e = Env::new();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (0, 0, 0));
}

#[test]
fn a_hidden_or_archived_session_keeps_its_hours() {
    let mut e = Env::new();
    e.scan();
    e.db.conn
        .execute("UPDATE session SET hidden=1, archived=1", [])
        .unwrap();
    e.scan();
    assert_eq!(e.totals(), (15 * MIN, 3, 5));
}
