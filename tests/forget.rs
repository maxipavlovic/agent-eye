//! `eye forget`: sessions of every agent, in temporary homes with fake transcripts. Nothing here
//! touches a real agent folder or the person's eye home.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::agents::{self, Agents};
use agent_eye::archive;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::forget::{self, OwnCopy};
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use rusqlite::Connection;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const CLAUDE_A: &str = "a0000001-0000-4000-8000-000000000001";
const CLAUDE_B: &str = "b0000002-0000-4000-8000-000000000002";
const CODEX_A: &str = "019a6a00-1111-7aaa-8bbb-000000000001";
const AGY_A: &str = "a1111111-1111-4111-8111-111111111111";
const AGY_B: &str = "b2222222-2222-4222-8222-222222222222";
const OC_A: &str = "ses_v1old";
const OC_B: &str = "ses_v1new";

struct World {
    _tmp: tempfile::TempDir,
    claude: PathBuf,
    codex: PathBuf,
    agy: PathBuf,
    opencode: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

fn sql(c: &Connection, q: &str, p: impl rusqlite::Params) {
    c.execute(q, p).unwrap();
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let claude = fixtures::claude_home(tmp.path());
    let fix = |n: &str| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(n)
    };
    let codex = tmp.path().join("codex");
    fixtures::copy_dir(&fix("codex-home"), &codex);
    let agy = tmp.path().join("agy");
    fixtures::copy_dir(&fix("agy-home"), &agy);
    std::fs::remove_file(agy.join("databases.json")).unwrap();
    std::fs::create_dir_all(agy.join("conversations")).unwrap();
    for id in [AGY_A, AGY_B] {
        let c = Connection::open(agy.join(format!("conversations/{id}.db"))).unwrap();
        c.execute_batch(
            "CREATE TABLE gen_metadata (idx integer PRIMARY KEY, data blob, size integer NOT NULL DEFAULT 0);",
        )
        .unwrap();
    }
    let c = Connection::open(agy.join("conversation_summaries.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE conversation_summaries (conversation_id text PRIMARY KEY, title text NOT NULL DEFAULT '',
         last_modified_time datetime NOT NULL, workspace_uris text NOT NULL, parent_conversation_id text NOT NULL DEFAULT '');",
    )
    .unwrap();
    for (id, t) in [(AGY_A, "Quokka census"), (AGY_B, "Other chat")] {
        sql(
            &c,
            "INSERT INTO conversation_summaries VALUES(?1,?2,'2026-11-05 14:05:10.123456+00:00','[\"file:///work/zoo\"]','')",
            (id, t),
        );
    }
    drop(c);
    let opencode = tmp.path().join("opencode");
    std::fs::create_dir_all(&opencode).unwrap();
    let c = Connection::open(opencode.join("opencode.db")).unwrap();
    c.execute_batch(
        "CREATE TABLE session (id text PRIMARY KEY, project_id text, directory text NOT NULL, parent_id text, title text, time_created integer, time_updated integer);
         CREATE TABLE message (id text PRIMARY KEY, session_id text NOT NULL, time_created integer, data text NOT NULL);
         CREATE TABLE part (id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer, data text NOT NULL);",
    )
    .unwrap();
    for (id, mid, text) in [
        (OC_A, "m_a", "Ping the old server"),
        (OC_B, "m_b", "Ping the new one"),
    ] {
        sql(
            &c,
            "INSERT INTO session VALUES(?1,'p','/work/oc',NULL,'t',1792000000000,1792000001000)",
            [id],
        );
        sql(
            &c,
            "INSERT INTO message VALUES(?1,?2,1792000000000,?3)",
            (
                mid,
                id,
                json!({"role":"user","time":{"created":1792000000000i64}}).to_string(),
            ),
        );
        sql(
            &c,
            "INSERT INTO part VALUES(?1,?2,?3,1,?4)",
            (
                format!("p_{mid}"),
                mid,
                id,
                json!({"type":"text","text":text}).to_string(),
            ),
        );
    }
    drop(c);
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    paths.ensure_home().unwrap();
    let mut cfg = Config::parse(&format!(
        "[agents]\nenabled = [\"claude\", \"codex\", \"agy\", \"opencode\"]\n[agents.codex]\ndir = \"{}\"\n[agents.agy]\ndir = \"{}\"\n[agents.opencode]\ndir = \"{}\"\n",
        codex.display(),
        agy.display(),
        opencode.display()
    ))
    .unwrap();
    cfg.override_agent_dir("claude", claude.clone());
    let db = Db::open(&paths.db_path()).unwrap();
    let mut w = World {
        _tmp: tmp,
        claude,
        codex,
        agy,
        opencode,
        paths,
        cfg,
        db,
    };
    scan(&mut w.db, &w.cfg, false).unwrap();
    archive::archive_all(&w.db, &w.paths, &w.agents()).unwrap();
    w
}

impl World {
    fn agents(&self) -> Agents {
        agents::enabled(&self.cfg)
    }
    fn has(&self, id: &str) -> bool {
        self.db.session_by_id(id).unwrap().is_some()
    }
    fn rows(&self, table: &str, id: &str) -> i64 {
        self.db
            .conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE session_id=?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn forget(&self, ids: &[&str], transcript: bool) -> anyhow::Result<forget::Done> {
        let s = forget::by_ids(
            &self.db,
            &ids.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )?;
        forget::forget(
            &self.db,
            &self.paths,
            &self.cfg,
            &self.agents(),
            &s,
            transcript,
        )
    }
    fn archive_of(&self, id: &str) -> Option<PathBuf> {
        let s = self.db.session_by_id(id).unwrap().unwrap();
        forget::archive_copy(&s, &self.paths, &self.agents())
    }
}

fn snapshots(w: &World) -> usize {
    std::fs::read_dir(w.paths.home.join("backups"))
        .map(|d| {
            d.flatten()
                .filter(|e| e.file_name().to_string_lossy().contains("manual"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn all_four_agents_are_scanned_in_the_fixture_world() {
    let w = world();
    for id in [CLAUDE_A, CLAUDE_B, CODEX_A, AGY_A, AGY_B, OC_A, OC_B] {
        assert!(w.has(id), "{id} should be a session");
        assert!(
            w.archive_of(id).is_some(),
            "{id} should have an archive copy"
        );
    }
}

#[test]
fn forgetting_removes_the_rows_and_the_archive_and_leaves_the_agent_copy_and_other_sessions() {
    let w = world();
    let archive_a = w.archive_of(CLAUDE_A).unwrap();
    let archive_b = w.archive_of(CLAUDE_B).unwrap();
    assert!(w.rows("session_fts", CLAUDE_A) > 0);
    assert!(
        w.rows("worked_hour", CLAUDE_A) > 0
            || w.rows("usage_day", CLAUDE_A) > 0
            || w.rows("stream", CLAUDE_A) > 0
    );
    let transcript = PathBuf::from(
        w.db.session_by_id(CLAUDE_A)
            .unwrap()
            .unwrap()
            .transcript_path
            .unwrap(),
    );
    let cursor_rows = |db: &Db| -> i64 {
        db.conn
            .query_row("SELECT count(*) FROM cursor", [], |r| r.get(0))
            .unwrap()
    };
    let cursors_before = cursor_rows(&w.db);
    let before = snapshots(&w);
    let done = w.forget(&[CLAUDE_A], false).unwrap();
    assert_eq!(done.outcomes.len(), 1);
    assert_eq!(done.outcomes[0].own, OwnCopy::Kept);
    assert!(!w.has(CLAUDE_A));
    for t in [
        "session_fts",
        "stream",
        "output",
        "event",
        "worked_hour",
        "worked_state",
        "usage_day",
        "usage_state",
        "loose",
        "jira_cand",
    ] {
        assert_eq!(w.rows(t, CLAUDE_A), 0, "{t}");
    }
    assert!(!archive_a.exists(), "eye's archive copy goes");
    assert!(archive_b.exists(), "another session's copy stays");
    assert!(
        transcript.exists(),
        "without --transcript the agent's file stays"
    );
    assert!(w.has(CLAUDE_B) && w.rows("session_fts", CLAUDE_B) > 0);
    assert!(cursor_rows(&w.db) < cursors_before, "its read cursors go");
    assert_eq!(
        snapshots(&w),
        before + 1,
        "a manual snapshot is taken first"
    );
    assert!(done.snapshot.exists());
    // the file is still there, so the next scan finds the session again
    let mut again = Db::open(&w.paths.db_path()).unwrap();
    scan(&mut again, &w.cfg, false).unwrap();
    assert!(again.session_by_id(CLAUDE_A).unwrap().is_some());
    // (an agent copy that stays brings the session back at the next scan)
}

#[test]
fn with_transcript_claude_loses_its_file_and_sub_agent_folder_only() {
    let w = world();
    let s = w.db.session_by_id(CLAUDE_A).unwrap().unwrap();
    let file = PathBuf::from(s.transcript_path.unwrap());
    let subs = file.with_extension("");
    assert!(subs.join("subagents").is_dir());
    let other = PathBuf::from(
        w.db.session_by_id(CLAUDE_B)
            .unwrap()
            .unwrap()
            .transcript_path
            .unwrap(),
    );
    let done = w.forget(&[CLAUDE_A], true).unwrap();
    assert!(matches!(&done.outcomes[0].own, OwnCopy::Removed(g) if g.len() == 2));
    assert!(!file.exists() && !subs.exists());
    assert!(other.exists());
    assert!(!w.has(CLAUDE_A) && w.has(CLAUDE_B));
}

#[test]
fn with_transcript_codex_loses_its_rollout_and_its_index_line() {
    let w = world();
    let index = w.codex.join("session_index.jsonl");
    let named = std::fs::read_to_string(&index).unwrap();
    assert!(named.contains(CODEX_A));
    let file_of = |id: &str| {
        PathBuf::from(
            w.db.session_by_id(id)
                .unwrap()
                .unwrap()
                .transcript_path
                .unwrap(),
        )
    };
    let other: String =
        w.db.conn
            .query_row(
                "SELECT id FROM session WHERE agent='codex' AND id<>?1",
                [CODEX_A],
                |r| r.get(0),
            )
            .unwrap();
    let (a, b) = (file_of(CODEX_A), file_of(&other));
    let done = w.forget(&[CODEX_A], true).unwrap();
    assert!(
        matches!(&done.outcomes[0].own, OwnCopy::Removed(g) if g.iter().any(|l| l.contains("session_index.jsonl")))
    );
    assert!(!a.exists() && b.exists());
    let after = std::fs::read_to_string(&index).unwrap_or_default();
    assert!(!after.contains(CODEX_A));
    assert_eq!(after.lines().count(), named.lines().count() - 1);
}

#[test]
fn with_transcript_agy_loses_folder_databases_and_summary_row() {
    let w = world();
    let brain = |id: &str| w.agy.join("brain").join(id);
    let db = |id: &str| w.agy.join(format!("conversations/{id}.db"));
    std::fs::write(format!("{}-wal", db(AGY_A).display()), b"x").unwrap();
    std::fs::write(format!("{}-shm", db(AGY_A).display()), b"x").unwrap();
    assert!(brain(AGY_A).is_dir());
    w.forget(&[AGY_A], true).unwrap();
    assert!(!brain(AGY_A).exists() && !db(AGY_A).exists());
    assert!(!w.agy.join(format!("conversations/{AGY_A}.db-wal")).exists());
    assert!(!w.agy.join(format!("conversations/{AGY_A}.db-shm")).exists());
    assert!(brain(AGY_B).is_dir() && db(AGY_B).is_file());
    let c = Connection::open(w.agy.join("conversation_summaries.db")).unwrap();
    let n = |id: &str| -> i64 {
        c.query_row(
            "SELECT count(*) FROM conversation_summaries WHERE conversation_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!((n(AGY_A), n(AGY_B)), (0, 1));
}

#[test]
fn with_transcript_opencode_loses_its_rows_only() {
    let w = world();
    w.forget(&[OC_A], true).unwrap();
    let c = Connection::open(w.opencode.join("opencode.db")).unwrap();
    let count = |t: &str, col: &str, id: &str| -> i64 {
        c.query_row(
            &format!("SELECT count(*) FROM {t} WHERE {col}=?1"),
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    for (t, col) in [
        ("session", "id"),
        ("message", "session_id"),
        ("part", "session_id"),
    ] {
        assert_eq!(count(t, col, OC_A), 0, "{t}");
        assert_eq!(count(t, col, OC_B), 1, "{t}");
    }
    assert!(!w.has(OC_A) && w.has(OC_B));
}

#[test]
fn with_transcript_opencode_json_files_go() {
    let w = world();
    let json_home = w.opencode.join("storage");
    fixtures::copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/opencode-home/storage"),
        &json_home,
    );
    let mut db = Db::open(&w.paths.db_path()).unwrap();
    scan(&mut db, &w.cfg, true).unwrap();
    assert!(db.session_by_id("ses_json1").unwrap().is_some());
    let s = forget::by_ids(&db, &["ses_json1".to_string()]).unwrap();
    forget::forget(&db, &w.paths, &w.cfg, &w.agents(), &s, true).unwrap();
    assert!(!json_home.join("session/prj1/ses_json1.json").exists());
    assert!(!json_home.join("message/ses_json1").exists());
    assert!(!json_home.join("part/msg_j1").exists() && !json_home.join("part/msg_j2").exists());
}

#[test]
fn a_session_that_is_open_now_is_refused_and_nothing_changes() {
    let w = world();
    sql(
        &w.db.conn,
        "UPDATE session SET live_status='idle' WHERE id=?1",
        [CLAUDE_B],
    );
    let before = snapshots(&w);
    let e = w
        .forget(&[CLAUDE_A, CLAUDE_B], true)
        .unwrap_err()
        .to_string();
    assert!(e.contains("open now") && e.contains(CLAUDE_B), "{e}");
    assert!(w.has(CLAUDE_A) && w.has(CLAUDE_B));
    assert_eq!(snapshots(&w), before, "no snapshot for a refused request");
    let file =
        w.db.session_by_id(CLAUDE_A)
            .unwrap()
            .unwrap()
            .transcript_path
            .unwrap();
    assert!(Path::new(&file).exists());
}

#[test]
fn an_adapter_that_cannot_delete_says_so_and_the_session_still_goes_from_eye() {
    let w = world();
    // the Claude adapter is asked about a path outside its folder: refused, and the row stays
    sql(
        &w.db.conn,
        "UPDATE session SET transcript_path='/etc/hosts' WHERE id=?1",
        [CLAUDE_B],
    );
    let done = w.forget(&[CLAUDE_B], true).unwrap();
    assert!(matches!(done.outcomes[0].own, OwnCopy::Failed(_)));
    assert!(!done.outcomes[0].forgotten && w.has(CLAUDE_B));
    assert!(Path::new("/etc/hosts").exists());
}

#[test]
fn a_forgotten_session_does_not_come_back_from_the_history_file() {
    let w = world();
    let history = w.claude.join("history.jsonl");
    let line = json!({"display":"Remember the milk","timestamp":1_793_000_000_000i64,"project":"/work/milk","sessionId":"f0000009-0000-4000-8000-000000000009"});
    std::fs::write(&history, format!("{line}\n")).unwrap();
    let mut db = Db::open(&w.paths.db_path()).unwrap();
    scan(&mut db, &w.cfg, true).unwrap();
    let id = "f0000009-0000-4000-8000-000000000009";
    assert!(
        db.session_by_id(id).unwrap().is_some(),
        "a history-only session"
    );
    let s = forget::by_ids(&db, &[id.to_string()]).unwrap();
    forget::forget(&db, &w.paths, &w.cfg, &w.agents(), &s, true).unwrap();
    scan(&mut db, &w.cfg, true).unwrap();
    assert!(db.session_by_id(id).unwrap().is_none());
}

#[test]
fn by_folder_picks_the_folder_and_what_is_inside_it() {
    let w = world();
    sql(
        &w.db.conn,
        "UPDATE session SET cwd='/work/billing/api' WHERE id=?1",
        [CLAUDE_A],
    );
    sql(
        &w.db.conn,
        "UPDATE session SET cwd='/work/billing' WHERE id=?1",
        [CLAUDE_B],
    );
    sql(
        &w.db.conn,
        "UPDATE session SET cwd='/work/billing-old' WHERE id=?1",
        [OC_A],
    );
    sql(
        &w.db.conn,
        "UPDATE session SET cwd='/elsewhere' WHERE id NOT IN (?1,?2,?3)",
        [CLAUDE_A, CLAUDE_B, OC_A],
    );
    let ids = |p: &str| -> Vec<String> {
        let mut v: Vec<String> = forget::by_folder(&w.db, Path::new(p))
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        ids("/work/billing"),
        vec![CLAUDE_A.to_string(), CLAUDE_B.to_string()]
    );
    assert_eq!(ids("/work/billing/api"), vec![CLAUDE_A.to_string()]);
    assert!(ids("/work/none").is_empty());
}

// ---- the command line ----

struct Cli {
    dir: tempfile::TempDir,
    claude: PathBuf,
}

impl Cli {
    fn new() -> Cli {
        let dir = tempfile::tempdir().unwrap();
        let claude = fixtures::claude_home(dir.path());
        let c = Cli { dir, claude };
        c.ok(&["scan"]);
        c
    }
    fn eye(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", self.dir.path())
            .env("EYE_HOME", self.dir.path().join("eye"))
            .env("EYE_CLAUDE_DIR", &self.claude)
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }
    fn sessions(&self) -> usize {
        let db = Db::open(&self.dir.path().join("eye/eye.db")).ok();
        db.map_or(0, |d| d.all_sessions().unwrap().len())
    }
}

#[test]
fn the_command_line_lists_asks_and_removes() {
    let c = Cli::new();
    let n = c.sessions();
    assert!(n >= 3, "{n}");
    let a = &CLAUDE_A[..8];
    let dry = c.ok(&["forget", a, "--dry-run"]);
    assert!(dry.contains(a) && dry.contains("dry run"), "{dry}");
    assert!(
        dry.contains("would remove") && !dry.contains("removed from eye"),
        "{dry}"
    );
    assert_eq!(c.sessions(), n, "a dry run changes nothing");
    let o = c.eye(&["forget", a]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("--yes"),
        "no terminal: say --yes"
    );
    assert_eq!(c.sessions(), n);
    let done = c.ok(&["forget", a, "--yes", "--transcript"]);
    assert!(
        done.contains("forgot") && done.contains("snapshot first"),
        "{done}"
    );
    assert_eq!(c.sessions(), n - 1);
    assert!(!c
        .claude
        .join(format!("projects/-work-billing/{CLAUDE_A}.jsonl"))
        .exists());
    assert!(c
        .claude
        .join(format!("projects/-work-billing/{CLAUDE_B}.jsonl"))
        .exists());
    let none = c.eye(&["forget"]);
    assert!(!none.status.success(), "ids or --folder are needed");
}

#[test]
fn the_command_line_forgets_a_folder() {
    let c = Cli::new();
    let n = c.sessions();
    let out = c.ok(&["forget", "--folder", "/work/billing", "--dry-run"]);
    assert!(out.contains("session"), "{out}");
    let before = out.lines().filter(|l| l.starts_with("  ")).count();
    assert!(before >= 2, "{out}");
    c.ok(&["forget", "--folder", "/work/billing", "--yes"]);
    assert_eq!(c.sessions(), n - before);
}

#[test]
fn a_disabled_agents_foreign_archive_path_falls_back_to_its_own_layout() {
    let w = world();
    let real = w.archive_of(OC_A).unwrap();
    // as after an import: the stored path is from another machine's home
    sql(
        &w.db.conn,
        "UPDATE session SET archive_path='/old-machine/.agent-eye/archive/@opencode/x/y.zst' WHERE id=?1",
        [OC_A],
    );
    let only_claude = Config::parse(&format!(
        "[agents]\nenabled = [\"claude\"]\n[agents.opencode]\ndir = \"{}\"\n",
        w.opencode.display()
    ))
    .unwrap();
    let agents = agents::enabled(&only_claude);
    assert!(agents
        .of_session(&w.db.session_by_id(OC_A).unwrap().unwrap())
        .is_none());
    let s = forget::by_ids(&w.db, &[OC_A.to_string()]).unwrap();
    let done = forget::forget(&w.db, &w.paths, &only_claude, &agents, &s, false).unwrap();
    assert!(done.outcomes[0].forgotten && done.outcomes[0].archive_removed);
    assert!(!real.exists(), "the copy at the agent's own layout goes");
    // a session with no copy anywhere says so
    let s = forget::by_ids(&w.db, &[OC_B.to_string()]).unwrap();
    std::fs::remove_file(w.archive_of(OC_B).unwrap()).ok();
    let done = forget::forget(&w.db, &w.paths, &only_claude, &agents, &s, false).unwrap();
    assert!(done.outcomes[0].forgotten && !done.outcomes[0].archive_removed);
}

#[test]
fn the_archive_file_stays_when_the_database_change_fails() {
    let w = world();
    let copy = w.archive_of(CLAUDE_A).unwrap();
    sql(
        &w.db.conn,
        "CREATE TRIGGER no_delete BEFORE DELETE ON session BEGIN SELECT RAISE(ABORT, 'forced'); END",
        [],
    );
    assert!(w.forget(&[CLAUDE_A], false).is_err());
    assert!(copy.exists(), "no row change, so the copy stays");
    assert!(w.has(CLAUDE_A) && w.rows("session_fts", CLAUDE_A) > 0);
    sql(&w.db.conn, "DROP TRIGGER no_delete", []);
    assert!(w.forget(&[CLAUDE_A], false).unwrap().outcomes[0].archive_removed);
    assert!(!copy.exists());
}
