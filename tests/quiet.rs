//! `[board] quiet`: a session an app started through an agent's SDK, kept quiet, gets no summary
//! from the daemon or `eye summarize --all` and is never sent to the corrections pass, while an
//! explicit `eye summarize <id>` still writes one. The daemon reads the key from the file, so the config it started with does
//! not matter. The transcripts copy the shape `tests/filters.rs` uses; `sdk-py` is the entrypoint
//! the Agent SDK for Python writes.

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::summarize::{summarize, Options};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const WORK: &str = "/work/p";
const SHOP: &str = "/work/shop";

/// A stand-in agent: it writes what it was asked to a log and answers with a summary.
const STUB: &str = r#"for last; do :; done
printf '%s\n' "$last" >> "$(dirname "$0")/calls.log"
echo '{"structured_output":{"status":"needs_you","state":"asked","next":"answer it","topics":[]}}'"#;

fn id(n: u32) -> String {
    format!("bbbbbbbb-0000-4000-8000-{n:012}")
}

/// Session `n` talks in hour `10 + n`: two sessions with turns at the same moment would read as a
/// fork and its parent, whose shared turns count once.
fn session(claude: &Path, n: u32, cwd: &str, entrypoint: &str, marker: &str) {
    let dir = claude.join(format!("projects/{}", cwd.replace('/', "-")));
    std::fs::create_dir_all(&dir).unwrap();
    let line = |role: &str, minute: u32, content: &str| {
        format!(
            r#"{{"type":"{role}","sessionId":"{}","cwd":"{cwd}","entrypoint":"{entrypoint}","timestamp":"2026-11-01T{:02}:{minute:02}:00.000Z","message":{{"role":"{role}","content":{content}}}}}"#,
            id(n),
            10 + n
        )
    };
    let text = [
        line("user", 0, &format!(r#""place the order, {marker}""#)),
        line("assistant", 1, r#"[{"type":"text","text":"done"}]"#),
        line("user", 2, &format!(r#""no, that is wrong, {marker}""#)),
        line(
            "assistant",
            3,
            r#"[{"type":"text","text":"Shall I place it now?"}]"#,
        ),
    ]
    .join("\n");
    std::fs::write(dir.join(format!("{}.jsonl", id(n))), text + "\n").unwrap();
}

fn options(ids: Vec<String>) -> Options {
    Options {
        ids,
        all: false,
        dry_run: false,
        max: None,
        force: true,
        print_excerpt: false,
    }
}

#[test]
fn a_quiet_session_gets_no_summary_from_the_daemon_or_a_bulk_run_and_stays_out_of_the_pass() {
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
    // the daemon started before the switch: its own config has nothing quiet
    let text = "[agents]\nenabled = [\"claude\"]\n\n[summary]\nauto_days = 36500\n";
    let mut started = Config::parse(text).unwrap();
    started.override_agent_dir("claude", claude.clone());
    let mut db = Db::open(&paths.db_path()).unwrap();
    session(&claude, 1, WORK, "cli", "mark-own");
    session(&claude, 2, SHOP, "sdk-py", "mark-app");
    scan(&mut db, &started, false).unwrap();
    let prompts: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM turn WHERE session_id=?1 AND kind='prompt'",
            [id(2)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prompts, 2, "the app's turns are recorded");
    let log = |n: &str| std::fs::read_to_string(bin.join("calls.log")).is_ok_and(|t| t.contains(n));

    // switched on in the file, as the board's gear does
    std::fs::write(
        paths.config_path(),
        format!("{text}\n[board]\nquiet = [\"app\"]\n"),
    )
    .unwrap();
    let quiet = started.fresh(&paths.config_path());
    let agents = agent_eye::agents::enabled(&quiet);
    agent_eye::clock::pass::run(&db, &paths, &quiet, &agents, 1_793_600_000_000, &mut |_| {})
        .unwrap();
    assert!(log("mark-own"), "the pass was asked about the user's own");
    assert!(!log("mark-app"), "the pass was asked about a quiet session");
    let (summarized, _) = agent_eye::daemon::quiet_round(&mut db, &paths, &started).unwrap();
    assert_eq!(summarized, 1, "the user's own session only");
    assert!(!log("mark-app"), "the daemon summarized a quiet session");
    let summary_of = |db: &Db, n: u32| db.session_by_id(&id(n)).unwrap().unwrap().summary_at;
    assert!(summary_of(&db, 1).is_some() && summary_of(&db, 2).is_none());

    // nor does a bulk run, which says why
    std::fs::remove_file(bin.join("calls.log")).unwrap();
    let mut all = options(vec![]);
    all.all = true;
    let out = summarize(&db, &paths, &quiet, &all).unwrap();
    assert!(
        log("mark-own"),
        "the bulk run did not summarize the user's own"
    );
    assert!(!log("mark-app"), "the bulk run summarized a quiet session");
    let app = out.iter().find(|o| o.id == id(2)).unwrap();
    assert_eq!(app.result, "skipped: quiet");

    // asked for by id, it is summarized all the same, in a bulk run too
    let mut both = options(vec![id(2)]);
    both.all = true;
    summarize(&db, &paths, &quiet, &both).unwrap();
    assert!(log("mark-app"));
    assert!(summary_of(&db, 2).is_some());

    // switched off again, the pass asks about it at its next run
    std::fs::write(paths.config_path(), text).unwrap();
    std::fs::remove_file(bin.join("calls.log")).unwrap();
    let loud = started.fresh(&paths.config_path());
    agent_eye::clock::pass::run(&db, &paths, &loud, &agents, 1_793_600_000_000, &mut |_| {})
        .unwrap();
    assert!(log("mark-app"), "the pass did not ask once it was loud");
    std::env::remove_var("EYE_CLAUDE_BIN");
}
