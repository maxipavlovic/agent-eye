//! Sessions eye leaves out (an excluded folder, a temp folder, a folder excluded after it was
//! indexed) and runs that are not the user's (headless): every pass that reads sessions, the
//! scan, the back-fill, the corrections pass and the summaries, leaves them alone. The tables are
//! found from the schema, so a table added later is swept too.

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::summarize::{summarize, Options};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const WORK: &str = "/work/p";
const SECRET: &str = "/work/secret";
const LATER: &str = "/work/later";
const SCRATCH: &str = "/tmp/scratch-filters";

/// A stand-in agent: it writes what it was asked to a log and answers nothing useful.
const STUB: &str = r#"for last; do :; done
printf '%s\n' "$last" >> "$(dirname "$0")/calls.log"
echo '{"structured_output":{}}'"#;

fn id(n: u32) -> String {
    format!("aaaaaaaa-0000-4000-8000-{n:012}")
}

fn session(claude: &Path, n: u32, cwd: &str, entrypoint: &str, marker: &str) {
    let dir = claude.join(format!("projects/{}", cwd.replace('/', "-")));
    std::fs::create_dir_all(&dir).unwrap();
    let line = |role: &str, minute: u32, content: &str| {
        format!(
            r#"{{"type":"{role}","sessionId":"{}","cwd":"{cwd}","entrypoint":"{entrypoint}","timestamp":"2026-11-01T10:{minute:02}:00.000Z","message":{{"role":"{role}","content":{content}}}}}"#,
            id(n)
        )
    };
    let text = [
        line("user", 0, &format!(r#""fix the {marker}""#)),
        line("assistant", 1, r#"[{"type":"text","text":"done"}]"#),
        line("user", 2, &format!(r#""no, that is wrong, {marker}""#)),
        line("assistant", 3, r#"[{"type":"text","text":"sorry"}]"#),
    ]
    .join("\n");
    std::fs::write(dir.join(format!("{}.jsonl", id(n))), text + "\n").unwrap();
}

/// Rows per table (found from the schema) for the sessions in `ids`, the session table included.
fn rows(db: &Db, ids: &[u32]) -> Vec<(String, i64)> {
    let tables: Vec<String> = db
        .conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut out = vec![];
    for t in tables {
        let col = match t.as_str() {
            "session" => "id",
            _ if db
                .conn
                .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name='session_id'")
                .unwrap()
                .exists([&t])
                .unwrap() =>
            {
                "session_id"
            }
            _ => continue,
        };
        // the back-fill's note that it has been to a session: it is how an excluded one is not asked for again
        if t == "turn_state" {
            continue;
        }
        for n in ids {
            let count: i64 = db
                .conn
                .query_row(
                    &format!("SELECT count(*) FROM \"{t}\" WHERE {col}=?1"),
                    [id(*n)],
                    |r| r.get(0),
                )
                .unwrap();
            out.push((format!("{t} {n}"), count));
        }
    }
    out
}

#[test]
fn left_out_sessions_gain_no_rows_in_any_table_from_any_pass() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("claude"), format!("#!/bin/sh\n{STUB}\n")).unwrap();
    std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("EYE_CLAUDE_BIN", bin.join("claude"));
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    paths.ensure_home().unwrap();
    let mut cfg = Config::parse("[agents]\nenabled = [\"claude\"]\n").unwrap();
    cfg.override_agent_dir("claude", claude.clone());
    cfg.exclude.paths = vec![SECRET.into()];
    let mut db = Db::open(&paths.db_path()).unwrap();

    // 1 is the user's own; 2 and 3 are never indexed; 4 is indexed and excluded later; 5 is headless
    session(&claude, 1, WORK, "cli", "mark-own");
    session(&claude, 2, SECRET, "cli", "mark-excluded");
    session(&claude, 3, SCRATCH, "cli", "mark-temp");
    session(&claude, 4, LATER, "cli", "mark-later");
    session(&claude, 5, WORK, "sdk-cli", "mark-headless");
    scan(&mut db, &cfg, false).unwrap();
    assert!(rows(&db, &[1]).iter().any(|(_, n)| *n > 0));
    assert_eq!(rows(&db, &[2, 3]).iter().map(|r| r.1).sum::<i64>(), 0);
    cfg.exclude.paths.push(LATER.into());

    // the clock's other writers: the pass first, then the summaries; each is asked only about
    // sessions it may read
    let log = |n: &str| std::fs::read_to_string(bin.join("calls.log")).is_ok_and(|t| t.contains(n));
    db.conn.execute("DELETE FROM turn", []).unwrap();
    db.conn.execute("DELETE FROM turn_state", []).unwrap();
    let before = rows(&db, &[2, 3, 4]);
    agent_eye::turns::backfill::run_all(&mut db, &paths, &cfg).unwrap();
    scan(&mut db, &cfg, true).unwrap();
    let agents = agent_eye::agents::enabled(&cfg);
    agent_eye::clock::pass::run(&db, &paths, &cfg, &agents, 1_793_600_000_000, &mut |_| {})
        .unwrap();
    for gone in ["mark-excluded", "mark-temp", "mark-later", "mark-headless"] {
        assert!(!log(gone), "the pass was asked about {gone}");
    }
    summarize(
        &db,
        &paths,
        &cfg,
        &Options {
            ids: vec![],
            all: true,
            dry_run: false,
            max: None,
            force: true,
            print_excerpt: false,
        },
    )
    .unwrap();
    // the stand-in is on the path, and was asked about the user's own session
    assert!(log("mark-own"));
    for gone in ["mark-excluded", "mark-temp", "mark-later"] {
        assert!(!log(gone), "the summaries were asked about {gone}");
    }
    assert_eq!(rows(&db, &[2, 3, 4]), before);
    // headless: read for the record, never judged
    let judged: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM correction WHERE session_id=?1",
            [id(5)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(judged, 0);
    std::env::remove_var("EYE_CLAUDE_BIN");
}
