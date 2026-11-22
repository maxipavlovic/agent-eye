//! Where a session was started: read from the transcript's first lines, mapped to a few words,
//! backfilled for sessions indexed before it was kept, and used by `eye ls --origin`.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct World {
    tmp: tempfile::TempDir,
    claude: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

fn id(n: u32) -> String {
    format!("aaaaaaaa-0000-4000-8000-{n:012}")
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        std::fs::create_dir_all(claude.join("projects/-work-p")).unwrap();
        let paths = Paths {
            home: tmp.path().join("eye"),
        };
        paths.ensure_home().unwrap();
        // the test's sessions live in temp folders, which eye leaves out by default
        let mut cfg =
            Config::parse("[agents]\nenabled = [\"claude\"]\n[exclude]\ntemp = false\n").unwrap();
        cfg.override_agent_dir("claude", claude.clone());
        cfg.night_dir = Some(paths.home.join("night"));
        let db = Db::open(&paths.db_path()).unwrap();
        World {
            tmp,
            claude,
            paths,
            cfg,
            db,
        }
    }

    /// A session of one prompt; `entrypoint` None leaves the field out.
    fn session(&self, n: u32, entrypoint: Option<&str>, cwd: &Path) {
        let ep = entrypoint
            .map(|e| format!(r#""entrypoint":"{e}","#))
            .unwrap_or_default();
        let line = format!(
            r#"{{"type":"user","sessionId":"{}","cwd":"{}",{ep}"timestamp":"2026-11-01T10:00:00.000Z","message":{{"role":"user","content":"hello {n}"}}}}"#,
            id(n),
            cwd.display()
        );
        std::fs::write(
            self.claude
                .join(format!("projects/-work-p/{}.jsonl", id(n))),
            format!("{line}\n"),
        )
        .unwrap();
    }

    fn origin(&self, n: u32) -> (Option<String>, Option<String>) {
        let s = self.db.session_by_id(&id(n)).unwrap().unwrap();
        (s.origin, s.origin_raw)
    }
}

fn some(a: &str, b: &str) -> (Option<String>, Option<String>) {
    (Some(a.into()), Some(b.into()))
}

#[test]
fn a_scan_stores_where_each_session_started() {
    let mut w = World::new();
    let night = w.paths.home.join("night/1/wt");
    w.session(1, Some("cli"), Path::new("/work/p"));
    w.session(2, Some("sdk-ts"), Path::new("/work/p"));
    w.session(3, Some("claude-vscode"), Path::new("/work/p"));
    w.session(4, Some("sdk-cli"), &night);
    w.session(5, Some("sdk-cli"), Path::new("/work/p"));
    w.session(6, Some("brand-new-thing"), Path::new("/work/p"));
    w.session(7, None, Path::new("/work/p"));
    // the user's own folder that happens to be called night/7/wt is not a night run
    w.session(8, Some("sdk-cli"), &w.tmp.path().join("own/night/7/wt"));
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(w.origin(1), some("terminal", "cli"));
    assert_eq!(w.origin(2), some("app", "sdk-ts"));
    assert_eq!(w.origin(3), some("ide", "claude-vscode"));
    assert_eq!(w.origin(4), some("night", "sdk-cli"));
    assert_eq!(w.origin(5), some("headless", "sdk-cli"));
    // a value nobody knows is kept raw and maps to nothing; no value is nothing at all
    assert_eq!(w.origin(6), (None, Some("brand-new-thing".into())));
    assert_eq!(w.origin(7), (None, None));
    assert_eq!(w.origin(8), some("headless", "sdk-cli"));
}

#[test]
fn a_symlinked_eye_home_still_finds_its_night_runs() {
    let w = World::new();
    let real = w.paths.home.join("night");
    std::fs::create_dir_all(real.join("4/wt")).unwrap();
    let link = w.tmp.path().join("link");
    std::os::unix::fs::symlink(&w.paths.home, &link).unwrap();
    // the home was configured by the link, the session reports the real folder, and the reverse
    let by_link = link.join("night");
    let run = real.join("4/wt");
    assert!(agent_eye::night::is_night_folder(
        Some(&by_link),
        run.to_str()
    ));
    assert!(agent_eye::night::is_night_folder(
        Some(&real),
        link.join("night/4/wt").to_str()
    ));
    // a run folder that is gone is still told by its place
    assert!(agent_eye::night::is_night_folder(
        Some(&by_link),
        real.join("9/work/x").to_str()
    ));
    assert!(!agent_eye::night::is_night_folder(
        Some(&by_link),
        real.join("9/log").to_str()
    ));
}

#[test]
fn sessions_indexed_before_origin_are_filled_without_a_re_read() {
    let mut w = World::new();
    w.session(1, Some("sdk-py"), Path::new("/work/p"));
    w.session(2, Some("cli"), Path::new("/work/p"));
    let first = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(first.files_parsed, 2);
    // what a database from before schema 17 looks like after its migration
    w.db.conn
        .execute("UPDATE session SET origin=NULL, origin_raw=NULL", [])
        .unwrap();
    assert_eq!(w.origin(1), (None, None));
    let again = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(again.files_parsed, 0, "no transcript is read again");
    assert_eq!(w.origin(1), some("app", "sdk-py"));
    assert_eq!(w.origin(2), some("terminal", "cli"));
    let cursors: i64 =
        w.db.conn
            .query_row("SELECT count(*) FROM cursor", [], |r| r.get(0))
            .unwrap();
    assert_eq!(cursors, 2);
}

#[test]
fn a_later_read_keeps_the_origin() {
    let mut w = World::new();
    w.session(1, Some("sdk-ts"), Path::new("/work/p"));
    scan(&mut w.db, &w.cfg, false).unwrap();
    let p = w.claude.join(format!("projects/-work-p/{}.jsonl", id(1)));
    let mut text = std::fs::read_to_string(&p).unwrap();
    text.push_str(&format!(
        r#"{{"type":"user","sessionId":"{}","cwd":"/work/p","timestamp":"2026-11-01T10:05:00.000Z","message":{{"role":"user","content":"more"}}}}"#,
        id(1)
    ));
    text.push('\n');
    std::fs::write(&p, text).unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(w.origin(1), some("app", "sdk-ts"));
}

#[test]
fn ls_filters_by_origin_and_shows_it_in_json() {
    let mut w = World::new();
    w.session(1, Some("cli"), Path::new("/work/p"));
    w.session(2, Some("sdk-ts"), Path::new("/work/p"));
    scan(&mut w.db, &w.cfg, false).unwrap();
    // no replies yet: both sit in the archive column, which the listing asks for by name
    let eye = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", w.tmp.path())
            .env("EYE_HOME", &w.paths.home)
            .env("EYE_CLAUDE_DIR", &w.claude)
            .env("PATH", "/usr/bin:/bin")
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = eye(&[
        "ls", "--origin", "app", "--json", "--all", "--column", "archive",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], id(2));
    assert_eq!(rows[0]["origin"], "app");
    assert_eq!(rows[0]["origin_raw"], "sdk-ts");
    // nothing from that origin: a plain sentence, not "run scan first"
    let out = eye(&["ls", "--origin", "night", "--all", "--column", "archive"]);
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "no sessions started from 'night'"
    );
    // a word nobody uses
    let out = eye(&["ls", "--origin", "moon"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown origin 'moon'"));
}
