//! A Claude Code home that is already full of history: several projects, a session with
//! sub-agents, one that was compacted, a background session started by another one, a session
//! whose transcript is gone but whose copy is in the archive, a session that only changed the
//! model, and prompts of sessions that have no transcript any more. Everything must load with the
//! right columns, counts, search hits, tokens and outputs. The fixture is synthetic
//! (`tests/fixtures/claude-home`).

#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::archive;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::server::{new_state, router};
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::Value;
use std::path::PathBuf;
use tower::ServiceExt;

const A: &str = "a0000001-0000-4000-8000-000000000001"; // sub-agents, a PR, a commit, a job child
const B: &str = "b0000002-0000-4000-8000-000000000002"; // compacted
const C: &str = "c0000003-0000-4000-8000-000000000003"; // only `/model`
const D: &str = "d0000004-0000-4000-8000-000000000004"; // the background job A started
const E: &str = "e0000005-0000-4000-8000-000000000005"; // transcript deleted, archive copy kept
const F: &str = "f0000006-0000-4000-8000-000000000006"; // history only
const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

struct World {
    _tmp: tempfile::TempDir,
    claude: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let claude = fixtures::claude_home(tmp.path());
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let cfg = Config::with_claude_dir(&claude);
    let db = Db::open(&paths.db_path()).unwrap();
    World {
        _tmp: tmp,
        claude,
        paths,
        cfg,
        db,
    }
}

impl World {
    fn app(&self) -> axum::Router {
        router(new_state(
            self.paths.clone(),
            self.cfg.clone(),
            TOKEN.into(),
            7420,
        ))
    }

    fn transcript(&self, project: &str, id: &str) -> PathBuf {
        self.claude
            .join("projects")
            .join(project)
            .join(format!("{id}.jsonl"))
    }
}

async fn get(app: &axum::Router, uri: &str) -> Value {
    let req = Request::builder()
        .uri(uri)
        .header("host", "127.0.0.1:7420")
        .header("x-eye-token", TOKEN)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.status().is_success(), "{uri}: {}", resp.status());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// `column id -> card ids` of a board.
fn by_column(board: &Value) -> Vec<(String, Vec<String>)> {
    board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["id"].as_str().unwrap().to_string(),
                c["cards"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k["id"].as_str().unwrap().to_string())
                    .collect(),
            )
        })
        .collect()
}

fn in_column(board: &Value, col: &str) -> Vec<String> {
    by_column(board)
        .into_iter()
        .find(|(c, _)| c == col)
        .unwrap()
        .1
}

#[tokio::test]
async fn everything_in_an_existing_home_loads() {
    let mut w = world();
    let st = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(st.files_seen, 5, "five transcripts in three projects");
    assert_eq!(st.history_prompts, 3);
    let s = |id: &str| w.db.session_by_id(id).unwrap().unwrap();

    // the session with sub-agents, a commit, a PR and a Jira key from its branch
    let a = s(A);
    assert_eq!(a.title.as_deref(), Some("Fix invoice rounding"));
    assert_eq!(
        (a.prompt_count, a.cwd.as_deref()),
        (2, Some("/work/billing"))
    );
    assert_eq!(a.git_branch.as_deref(), Some("bugfix/BILL-101"));
    let outs: Vec<(String, String)> =
        w.db.outputs_for(A)
            .unwrap()
            .into_iter()
            .map(|o| (o.kind, o.value))
            .collect();
    for want in [
        ("pr", "https://github.com/acme/billing/pull/42"),
        ("commit", "abc1234"),
        ("jira", "BILL-101"),
    ] {
        assert!(
            outs.contains(&(want.0.into(), want.1.into())),
            "{want:?} missing in {outs:?}"
        );
    }
    // tokens: both assistant messages of the main transcript, then the two sub-agents
    let t = a.tokens.unwrap();
    assert_eq!(
        (t.input, t.output, t.cache_write, t.cache_read),
        (110, 70, 200, 700)
    );
    assert_eq!(
        (t.main_total(), t.subagents_total, t.total()),
        (1080, 102, 1182)
    );
    let kids: Vec<(String, String)> =
        w.db.streams_for(A)
            .unwrap()
            .into_iter()
            .filter(|x| x.kind == "child")
            .map(|x| (x.title.unwrap_or_default(), x.status.unwrap_or_default()))
            .collect();
    assert!(kids.contains(&("Review the rounding change".into(), "done".into())));
    assert!(kids.contains(&("Look for other rounding calls".into(), "running".into())));

    // the compacted session: the summary line is not a prompt, the recap is kept
    let b = s(B);
    assert_eq!(b.title.as_deref(), Some("Reports module migration"));
    assert_eq!(b.prompt_count, 3, "compaction summary is not a prompt");
    assert!(b
        .recap
        .as_deref()
        .unwrap()
        .starts_with("Goal: migrate the reports module"));
    assert!(!b.recap.as_deref().unwrap().contains("disable recaps"));
    assert_eq!(
        b.last_assistant_text.as_deref(),
        Some("Adapter tests written; two still fail on null dates.")
    );

    // the background session is a child of the session that started it
    let d = s(D);
    assert_eq!(d.kind.as_deref(), Some("bg"));
    assert_eq!(d.parent_session_id.as_deref(), Some(A));
    assert_eq!(
        d.title.as_deref(),
        Some("Release notes"),
        "the job name beats the prompt"
    );
    assert!(d
        .recap
        .as_deref()
        .unwrap()
        .contains("Release notes drafted"));
    let a_kids = w.db.streams_for(A).unwrap();
    assert!(a_kids
        .iter()
        .any(|x| x.kind == "child" && x.key == D && x.status.as_deref() == Some("done")));

    // history-only: its prompts survive, it cannot be resumed
    let f = s(F);
    assert_eq!(f.kind.as_deref(), Some("history_only"));
    assert_eq!(
        (f.prompt_count, f.cwd.as_deref()),
        (2, Some("/work/staging"))
    );

    // search: transcript text, titles, history prompts; the `/model` session is hidden
    let find = |words: &str, all: bool| -> Vec<String> {
        let ws: Vec<String> = words.split(' ').map(String::from).collect();
        let hidden = if all {
            Default::default()
        } else {
            agent_eye::hide::hidden_ids(&w.db, &w.cfg).unwrap()
        };
        agent_eye::search::find_without(&w.db, &ws, 10, all, &hidden, agent_eye::util::now_ms())
            .unwrap()
            .into_iter()
            .map(|h| h.session.id)
            .collect()
    };
    assert_eq!(find("rounding", false)[0], A);
    assert_eq!(find("adapter tests", false), [B]);
    assert_eq!(find("zebra", true), [F]);
    assert_eq!(find("BILL-101", false)[0], A);
    assert!(
        find("opus", false).is_empty(),
        "a /model-only session is hidden"
    );
    assert_eq!(find("opus", true), [C]);

    // the board: columns and counts
    w.db.conn
        .execute("UPDATE session SET live_status='busy' WHERE id=?1", [B])
        .unwrap();
    let app = w.app();
    let board = get(&app, "/api/board").await;
    assert_eq!(in_column(&board, "running"), [B]);
    let paused = in_column(&board, "paused");
    assert!(
        paused.contains(&A.to_string()) && paused.contains(&E.to_string()),
        "{paused:?}"
    );
    let on_board: Vec<String> = by_column(&board).into_iter().flat_map(|(_, v)| v).collect();
    assert!(!on_board.contains(&C.to_string()), "hidden");
    assert!(
        !on_board.contains(&F.to_string()),
        "history-only goes to the archive"
    );
    assert!(board["archive_count"].as_u64().unwrap() >= 1);
    let all = get(&app, "/api/archive?all=1").await;
    let archived: Vec<&str> = all["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(archived.contains(&F) && archived.contains(&C));
    // cards carry their counts and outputs
    let card = get(&app, &format!("/api/cards/{A}")).await;
    assert_eq!(card["agent"], "claude");
    assert_eq!(
        card["child_count"], 3,
        "two sub-agents and the background session"
    );
    assert_eq!(card["tokens"]["total"], 1182);
    assert_eq!(card["tokens"]["subagents_total"], 102);
    assert_eq!(
        card["resume_cmd"],
        format!("cd '/work/billing' && claude --resume {A}")
    );
    let kinds: Vec<&str> = card["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"pr") && kinds.contains(&"commit"));
    // a second scan changes nothing
    let again = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(again.files_parsed, 0);
}

#[tokio::test]
async fn a_session_whose_transcript_was_deleted_is_kept_by_the_archive() {
    let mut w = world();
    scan(&mut w.db, &w.cfg, false).unwrap();
    let agents = agent_eye::agents::enabled(&w.cfg);
    assert!(matches!(
        archive::archive_session(&w.db, &w.paths, &agents, E).unwrap(),
        archive::Archived::Copied(_)
    ));
    let original = std::fs::read(w.transcript("-work-gone", E)).unwrap();
    std::fs::remove_file(w.transcript("-work-gone", E)).unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    let e = w.db.session_by_id(E).unwrap().unwrap();
    assert_eq!(e.transcript_state.as_deref(), Some("archived"));
    // still on the board, still searchable, and its conversation is read from the archive copy
    let app = w.app();
    let board = get(&app, "/api/board").await;
    assert!(in_column(&board, "paused").contains(&E.to_string()));
    let hit = get(&app, "/api/search?q=nightly+export").await;
    assert_eq!(hit["results"][0]["card"]["id"], E);
    let talk = get(&app, &format!("/api/cards/{E}/conversation")).await;
    assert_eq!(talk["messages"].as_array().unwrap().len(), 4);
    assert_eq!(
        talk["messages"][3]["text"],
        "Simplified: one pass, fail loudly."
    );
    // and resuming puts the transcript back where Claude Code expects it
    let back = archive::restore(&w.db, &w.paths, &agents, E).unwrap();
    assert_eq!(back, w.transcript("-work-gone", E));
    assert_eq!(std::fs::read(back).unwrap(), original);
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(
        w.db.session_by_id(E)
            .unwrap()
            .unwrap()
            .transcript_state
            .as_deref(),
        Some("present")
    );
}

#[test]
fn the_terminal_picker_lists_what_the_scan_found() {
    let w = world();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_agent-eye"))
        .args(["pick", "--filter", "rounding"])
        .env("EYE_CLAUDE_DIR", &w.claude)
        .env("EYE_HOME", &w.paths.home)
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && text.contains("Fix invoice rounding"),
        "{text}"
    );
    let ls = std::process::Command::new(env!("CARGO_BIN_EXE_agent-eye"))
        .args(["ls", "--json"])
        .env("EYE_CLAUDE_DIR", &w.claude)
        .env("EYE_HOME", &w.paths.home)
        .output()
        .unwrap();
    let rows: Value = serde_json::from_slice(&ls.stdout).unwrap();
    let ids: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&A) && ids.contains(&B) && ids.contains(&D));
    assert!(
        !ids.contains(&C),
        "the /model session is hidden from the list"
    );
}
