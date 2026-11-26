//! Turn events from the files to the database: the normal read, a transcript that is rewritten, the
//! backfill of sessions an older eye already read, and the rule that keeps an event from being
//! stored twice.

use agent_eye::agents::Agents;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::turns::backfill;
use std::io::Write;
use std::path::{Path, PathBuf};

const SID: &str = "eeeeeeee-0000-4000-8000-000000000005";
const FIXTURE: &str = include_str!("fixtures/claude-turns.jsonl");

struct Env {
    _tmp: tempfile::TempDir,
    transcript: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl Env {
    fn new(text: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        let proj = claude.join("projects/-tmp-demo");
        std::fs::create_dir_all(&proj).unwrap();
        let transcript = proj.join(format!("{SID}.jsonl"));
        std::fs::write(&transcript, text).unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        // the test's sessions live in temp folders, which eye leaves out by default
        cfg.exclude.temp = false;
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

    /// The database as it was before schema 18: sessions read, no turns.
    fn forget_turns(&self) {
        self.db.conn.execute("DELETE FROM turn", []).unwrap();
        self.db.conn.execute("DELETE FROM turn_state", []).unwrap();
    }

    fn backfill(&mut self) -> backfill::Pass {
        let agents = self.agents();
        backfill::run_all(&mut self.db, &self.paths, &agents).unwrap()
    }

    /// How many events of each kind the session has.
    fn counts(&self) -> Vec<(String, i64)> {
        self.db
            .conn
            .prepare(
                "SELECT kind, count(*) FROM turn WHERE session_id=?1 GROUP BY kind ORDER BY kind",
            )
            .unwrap()
            .query_map([SID], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn kinds(pairs: &[(&str, i64)]) -> Vec<(String, i64)> {
    pairs.iter().map(|(k, n)| (k.to_string(), *n)).collect()
}

/// What the fixture holds, by kind.
fn all_of_the_fixture() -> Vec<(String, i64)> {
    kinds(&[
        ("compact", 1),
        ("end", 2),
        ("interrupt", 1),
        ("prompt", 2),
        ("refusal", 1),
        ("think", 1),
        ("tool", 2),
        ("tool_error", 1),
    ])
}

#[test]
fn a_session_read_for_the_first_time_gets_its_turns_from_the_normal_read() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    assert_eq!(e.counts(), all_of_the_fixture());
    assert_eq!(backfill::waiting(&e.db, &e.agents()).unwrap(), 0);
}

#[test]
fn a_session_that_grows_gets_what_is_new_and_a_scan_that_finds_nothing_changes_nothing() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let (head, tail) = lines.split_at(14);
    let mut e = Env::new(&(head.join("\n") + "\n"));
    e.scan();
    e.scan();
    append(&e.transcript, &(tail.join("\n") + "\n"));
    e.scan();
    e.scan();
    // the first read closed the turn it stopped in, and the lines that continue it do not close it again
    let counts = e.counts();
    let n = |k: &str| counts.iter().find(|(c, _)| c == k).map_or(0, |(_, n)| *n);
    assert_eq!(
        (n("prompt"), n("end"), n("compact"), n("tool")),
        (2, 2, 1, 2)
    );
}

#[test]
fn a_full_scan_replaces_the_turns_instead_of_adding() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(e.counts(), all_of_the_fixture());
}

#[test]
fn a_rewritten_transcript_replaces_what_was_stored() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    // shorter than before: the adapter reads it again from the start
    let first = FIXTURE.lines().next().unwrap().to_string() + "\n";
    std::fs::write(&e.transcript, first).unwrap();
    e.scan();
    assert_eq!(
        e.counts(),
        kinds(&[("end", 0), ("prompt", 1)])[1..].to_vec()
    );
}

#[test]
fn an_old_session_gets_its_turns_from_its_transcript_and_nothing_else_changes() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    let cursor = |e: &Env| -> String {
        e.db.conn
            .query_row("SELECT marker FROM cursor", [], |r| r.get(0))
            .unwrap()
    };
    let before = cursor(&e);
    e.forget_turns();
    assert_eq!(backfill::waiting(&e.db, &e.agents()).unwrap(), 1);
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (1, 0, 0));
    assert_eq!(e.counts(), all_of_the_fixture());
    assert_eq!(cursor(&e), before);
    // done once: another pass finds nothing, and writes the mark
    assert_eq!(e.backfill().counted, 0);
    assert!(e.db.meta_get(backfill::DONE_KEY).unwrap().is_some());
    // a normal read of the grown file adds only what is new
    append(
        &e.transcript,
        &(FIXTURE.lines().nth(4).unwrap().to_string() + "\n"),
    );
    e.scan();
    let tool_errors = e.counts().into_iter().find(|(k, _)| k == "tool_error");
    assert_eq!(tool_errors, Some(("tool_error".to_string(), 2)));
}

#[test]
fn a_session_not_counted_yet_is_left_to_the_backfill_when_it_grows() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    e.forget_turns();
    append(
        &e.transcript,
        &(FIXTURE.lines().nth(4).unwrap().to_string() + "\n"),
    );
    e.scan();
    assert!(
        e.counts().is_empty(),
        "the backfill reads the whole file, so the read adds nothing"
    );
    e.backfill();
    assert_eq!(
        e.counts()
            .iter()
            .find(|(k, _)| k == "tool_error")
            .unwrap()
            .1,
        2
    );
}

#[test]
fn a_session_whose_transcript_is_gone_waits_for_nothing() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    e.forget_turns();
    std::fs::remove_file(&e.transcript).unwrap();
    e.scan();
    assert_eq!(backfill::waiting(&e.db, &e.agents()).unwrap(), 0);
    assert_eq!(e.backfill().counted, 0);
}

#[test]
fn an_empty_home_has_nothing_to_backfill() {
    let mut e = Env::new("");
    e.scan();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (0, 0, 0));
}
