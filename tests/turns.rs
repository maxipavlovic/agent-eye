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
        backfill::run_all(&mut self.db, &self.paths, &self.cfg).unwrap()
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
        ("end", 1),
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
    // the scanner and the turn reader agree on what a prompt is, and an end follows a prompt
    let prompts = e.counts().iter().find(|(k, _)| k == "prompt").unwrap().1;
    let counted: i64 =
        e.db.conn
            .query_row("SELECT prompt_count FROM session", [], |r| r.get(0))
            .unwrap();
    assert_eq!(counted, prompts);
    let kinds: Vec<String> =
        e.db.conn
            .prepare("SELECT kind FROM turn WHERE kind IN ('prompt','end') ORDER BY rowid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
    for pair in kinds.windows(2) {
        assert!(pair[0] != "end" || pair[1] != "end", "two ends in a row");
    }
    assert_ne!(kinds.first().map(String::as_str), Some("end"));
}

/// A prompt typed while the agent worked and a comment on a refused call are prompts to the turn
/// reader, so the session's count has to include them.
#[test]
fn the_prompt_count_is_the_number_of_prompt_rows_with_queued_prompts_and_comments() {
    let said = "The user doesn't want to proceed with this tool use. To tell you how to proceed, the user said:\nuse rg instead";
    let comment = serde_json::json!({"type": "user", "timestamp": "2026-11-02T10:08:00.000Z",
        "message": {"content": [{"type": "tool_result", "tool_use_id": "t9", "is_error": true, "content": said}]}});
    // a queued prompt as the agent writes it: the type is not the first key
    let queued = r#"{"attachment":{"type":"queued_command","prompt":"and the tests","commandMode":"prompt","origin":{"kind":"human"}},"timestamp":"2026-11-02T10:09:00.000Z","type":"attachment"}"#;
    let mut e = Env::new(&format!("{FIXTURE}{comment}\n{queued}\n"));
    e.scan();
    let one = |sql: &str| -> i64 { e.db.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(one("SELECT count(*) FROM turn WHERE kind='prompt'"), 4);
    assert_eq!(one("SELECT prompt_count FROM session"), 4);
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
    // the turn the first read stopped in stays open: the last turn has no end until a prompt follows
    let counts = e.counts();
    let n = |k: &str| counts.iter().find(|(c, _)| c == k).map_or(0, |(_, n)| *n);
    assert_eq!(
        (n("prompt"), n("end"), n("compact"), n("tool")),
        (2, 1, 1, 2)
    );
}

/// The times of the session's `end` events, in order.
fn end_times(e: &Env) -> Vec<i64> {
    e.db.conn
        .prepare("SELECT ts FROM turn WHERE session_id=?1 AND kind='end' ORDER BY ts")
        .unwrap()
        .query_map([SID], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A prompt after the fixture's last reply (10:07:00).
const NEXT_PROMPT: &str = r#"{"type": "user", "timestamp": "2026-11-02T10:09:00.000Z", "cwd": "/tmp/demo", "sessionId": "eeeeeeee-0000-4000-8000-000000000005", "message": {"content": "thanks, now ship it"}}"#;

#[test]
fn a_turn_a_first_read_caught_in_the_middle_ends_once_at_its_last_reply() {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let (head, tail) = lines.split_at(14);
    let mut e = Env::new(&(head.join("\n") + "\n"));
    e.scan();
    append(
        &e.transcript,
        &(tail.join("\n") + "\n" + NEXT_PROMPT + "\n"),
    );
    e.scan();
    // the end of the second turn is its last reply (10:07:00), not the one the first read saw
    let ends = end_times(&e);
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[1] - ends[0], (7 * 60 - (2 * 60 + 10)) * 1000);
}

#[test]
fn a_session_active_during_the_upgrade_gets_one_end_for_its_open_turn() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    e.forget_turns();
    e.backfill();
    append(&e.transcript, &(NEXT_PROMPT.to_string() + "\n"));
    e.scan();
    assert_eq!(end_times(&e).len(), 2);
}

/// What a session that was read, lost its turns (as before the upgrade) and then grew holds when
/// the whole file is read at once.
fn whole_read() -> Vec<(String, i64)> {
    let mut e = Env::new(FIXTURE);
    e.scan();
    e.counts()
}

/// A session read up to line `at`, without turns, that then grew by the rest of the fixture.
fn grown_without_turns(at: usize) -> Env {
    let lines: Vec<&str> = FIXTURE.lines().collect();
    let (head, tail) = lines.split_at(at);
    let mut e = Env::new(&(head.join("\n") + "\n"));
    e.scan();
    e.forget_turns();
    append(&e.transcript, &(tail.join("\n") + "\n"));
    e
}

#[test]
fn a_session_that_grew_before_the_backfill_got_to_it_is_counted_once_whichever_comes_first() {
    let whole = whole_read();
    // wherever the first read stopped
    for at in 1..FIXTURE.lines().count() {
        // the scan first: it adds nothing to a session without turns, the backfill then reads it whole
        let mut e = grown_without_turns(at);
        e.scan();
        e.backfill();
        e.scan();
        assert_eq!(e.counts(), whole, "scan first, split after line {at}");

        // the backfill first: it waits for the scan if that has not read the new lines yet
        let mut e = grown_without_turns(at);
        e.backfill();
        e.scan();
        e.backfill();
        e.scan();
        assert_eq!(e.counts(), whole, "backfill first, split after line {at}");
    }
}

#[test]
fn a_session_in_a_folder_excluded_after_it_was_indexed_gets_no_turns_from_the_backfill() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    e.forget_turns();
    e.cfg.exclude.paths = vec!["/tmp/demo".into()];
    assert_eq!(e.backfill().counted, 1);
    assert_eq!(all_rows(&e), 0);
    assert_eq!(backfill::waiting(&e.db, &e.agents()).unwrap(), 0);
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

/// How many turn rows the database has, of any session.
fn all_rows(e: &Env) -> i64 {
    e.db.conn
        .query_row("SELECT count(*) FROM turn", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_session_in_an_excluded_folder_gets_no_turns() {
    let mut e = Env::new(FIXTURE);
    e.cfg.exclude.paths = vec!["/tmp/demo".into()];
    e.scan();
    assert_eq!(all_rows(&e), 0);
    assert_eq!(e.backfill().counted, 0);
    let rows: i64 =
        e.db.conn
            .query_row("SELECT count(*) FROM turn_state", [], |r| r.get(0))
            .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_session_in_a_temp_folder_gets_no_turns_unless_temp_folders_are_followed() {
    let mut e = Env::new(FIXTURE);
    e.cfg.exclude.temp = true;
    e.scan();
    assert_eq!(all_rows(&e), 0);
    e.cfg.exclude.temp = false;
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert!(
        all_rows(&e) > 0,
        "the session is indexed once it is allowed"
    );
}

#[test]
fn with_the_clock_off_nothing_new_is_recorded_and_what_is_there_stays() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    let before = all_rows(&e);
    e.cfg.clock.enabled = false;
    append(&e.transcript, &(NEXT_PROMPT.to_string() + "\n"));
    e.scan();
    scan(&mut e.db, &e.cfg, true).unwrap();
    assert_eq!(all_rows(&e), before);
    // the back-fill reads nothing either, and does not say it is done
    e.forget_turns();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable), (0, 0));
    assert_eq!(all_rows(&e), 0);
    assert!(e.db.meta_get(backfill::DONE_KEY).unwrap().is_none());
    // switched on again, the back-fill catches up
    e.cfg.clock.enabled = true;
    assert_eq!(e.backfill().counted, 1);
    assert!(all_rows(&e) > before);
}

#[test]
fn turns_of_sessions_outside_the_index_are_cleared_once_when_the_database_is_opened() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    let kept = all_rows(&e);
    e.db.conn
        .execute_batch(
            "INSERT INTO turn(session_id, ts, kind, n) VALUES('ghost', 1, 'prompt', 5);
             INSERT INTO turn_state(session_id) VALUES('ghost');
             DELETE FROM meta WHERE key='turns_unindexed_dropped';",
        )
        .unwrap();
    let db = Db::open(&e.paths.db_path()).unwrap();
    let ghosts: i64 = db
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM turn WHERE session_id='ghost')
                  + (SELECT count(*) FROM turn_state WHERE session_id='ghost')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let rows: i64 = db
        .conn
        .query_row("SELECT count(*) FROM turn", [], |r| r.get(0))
        .unwrap();
    assert_eq!((ghosts, rows), (0, kept));
}

#[test]
fn a_claude_session_whose_transcript_is_gone_gets_its_turns_from_eyes_archive_copy() {
    let mut e = Env::new(FIXTURE);
    e.scan();
    let id: String =
        e.db.conn
            .query_row("SELECT id FROM session", [], |r| r.get(0))
            .unwrap();
    agent_eye::archive::archive_session(&e.db, &e.paths, &e.agents(), &id).unwrap();
    e.forget_turns();
    std::fs::remove_file(&e.transcript).unwrap();
    e.scan();
    assert_eq!(backfill::waiting(&e.db, &e.agents()).unwrap(), 1);
    e.backfill();
    assert_eq!(e.counts(), all_of_the_fixture());
}
