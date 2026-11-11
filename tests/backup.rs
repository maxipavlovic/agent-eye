//! A restored snapshot is brought up to date by the next scan.

use agent_eye::backup::{self, Reason};
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::install::Opts;
use agent_eye::paths::Paths;
use agent_eye::platform::service::{Manager, Timing};
use agent_eye::scan::scan;

const S1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const S2: &str = "bbbbbbbb-0000-4000-8000-000000000002";

fn transcript(id: &str, prompt: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"2026-11-01T10:01:40.000Z\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{id}\",\"message\":{{\"content\":\"{prompt}\"}}}}\n{{\"type\":\"assistant\",\"timestamp\":\"2026-11-01T10:02:40.000Z\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{id}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Done\"}}]}}}}\n"
    )
}

fn sessions(db: &Db) -> i64 {
    db.conn
        .query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn restoring_an_older_snapshot_and_scanning_brings_back_the_newer_session() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let proj = claude.join("projects/-tmp-demo");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{S1}.jsonl")),
        transcript(S1, "First task"),
    )
    .unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
    cfg.override_agent_dir("claude", claude);

    let mut db = Db::open(&paths.db_path()).unwrap();
    scan(&mut db, &cfg, false).unwrap();
    assert_eq!(sessions(&db), 1);
    backup::take(&db.conn, &paths, &cfg.backup, &Reason::Manual).unwrap();

    std::fs::write(
        proj.join(format!("{S2}.jsonl")),
        transcript(S2, "Second task"),
    )
    .unwrap();
    scan(&mut db, &cfg, false).unwrap();
    assert_eq!(sessions(&db), 2);
    drop(db);

    let entry = backup::find(&backup::dir(&paths), "latest").unwrap();
    let prepared = backup::prepare(&paths, entry).unwrap();
    let o = Opts {
        eye_home: paths.home.clone(),
        bin_dir: paths.home.join("bin"),
        manager: Manager::current(),
        service_dir: paths.home.join("agents"),
        exe: std::env::current_exe().unwrap(),
        dry_run: false,
        no_service: true,
        purge: false,
        board: false,
        programs: None,
        timing: Timing::REAL,
    };
    backup::apply(&o, &cfg, &prepared).unwrap();

    let mut db = Db::open(&paths.db_path()).unwrap();
    assert_eq!(sessions(&db), 1, "the snapshot has only the first session");
    scan(&mut db, &cfg, false).unwrap();
    assert_eq!(
        sessions(&db),
        2,
        "the next scan reads the newer transcript again"
    );
}

/// Run the binary with its home and eye home in temp folders, and nothing on the terminal.
fn eye(t: &std::path::Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_agent-eye"))
        .env("HOME", t)
        .env("EYE_HOME", t.join("eye"))
        .env_remove("EYE_CLAUDE_DIR")
        .env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn text(o: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn the_commands_make_list_and_restore_snapshots() {
    let t = tempfile::tempdir().unwrap();
    let none = eye(t.path(), &["backup", "ls"]);
    assert!(text(&none).contains("no snapshots yet"), "{}", text(&none));
    let made = eye(t.path(), &["backup"]);
    assert!(made.status.success(), "{}", text(&made));
    assert!(text(&made).starts_with("snapshot "), "{}", text(&made));
    let ls = text(&eye(t.path(), &["backup", "ls"]));
    assert!(ls.contains("manual") && ls.lines().count() == 2, "{ls}");
    assert!(ls.trim_end().ends_with(&backup_schema()), "{ls}");

    let asked = eye(t.path(), &["backup", "restore", "latest"]);
    assert!(!asked.status.success());
    assert!(text(&asked).contains("--yes"), "{}", text(&asked));
    let done = eye(t.path(), &["backup", "restore", "latest", "--yes"]);
    assert!(done.status.success(), "{}", text(&done));
    assert!(
        text(&done).contains("next scan brings back"),
        "{}",
        text(&done)
    );
    let ls = text(&eye(t.path(), &["backup", "ls"]));
    assert!(ls.contains("pre-restore"), "{ls}");
}

fn backup_schema() -> String {
    agent_eye::db::SCHEMA_VERSION.to_string()
}

#[test]
fn a_manual_snapshot_says_when_automatic_ones_are_off() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("eye")).unwrap();
    std::fs::write(
        t.path().join("eye/config.toml"),
        "[backup]\nenabled = false\n",
    )
    .unwrap();
    let o = eye(t.path(), &["backup"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(
        text(&o).contains("automatic snapshots are off"),
        "{}",
        text(&o)
    );
}
