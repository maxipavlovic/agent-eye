//! The agent layer end to end: a second agent with its own file format, registered through the
//! same trait as Claude Code, shows up next to it in the scan, the board API, search, resume
//! commands, conversations, the archive and ideas. And Claude Code can be switched off.

#[path = "support/fake_agent.rs"]
mod fake_agent;

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::server::{new_state, router};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
const CLAUDE_ID: &str = "aaaaaaaa-0000-4000-8000-0000000000c1";
const FAKE_ID: &str = "fake-session-0001";

struct World {
    _tmp: tempfile::TempDir,
    claude: std::path::PathBuf,
    fake: std::path::PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl World {
    /// Change which agents are enabled; the database stays.
    fn enable(&mut self, enabled: &str) {
        self.cfg = config(enabled, &self.claude, &self.fake);
    }
}

fn config(enabled: &str, claude: &std::path::Path, fake: &std::path::Path) -> Config {
    let mut cfg = Config::parse(&format!(
        "[agents]\nenabled = {enabled}\n[agents.fake]\ndir = \"{}\"\n",
        fake.display()
    ))
    .unwrap();
    cfg.override_agent_dir("claude", claude.to_path_buf());
    cfg
}

/// Claude Code with one session and the fake agent with one, both in temp folders.
fn world(enabled: &str) -> World {
    fake_agent::register();
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let fake = tmp.path().join("fake");
    let proj = claude.join("projects/-work-claude");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(format!("{CLAUDE_ID}.jsonl")),
        format!(
            "{}\n{}\n{}\n",
            json!({"type":"user","timestamp":"2026-11-01T10:00:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"content":"Investigate the platypus report"}}),
            json!({"type":"assistant","timestamp":"2026-11-01T10:00:05Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"id":"m1","content":[{"type":"text","text":"The platypus report is fine."}]}}),
            json!({"type":"user","timestamp":"2026-11-01T10:01:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"content":"Thanks, close it"}}),
        ),
    )
    .unwrap();
    let logs = fake.join("logs/work-fake");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
        logs.join(format!("{FAKE_ID}.log")),
        "cwd|/work/fake\nu|1793527200000|Find the zebra crossing\na|1793527205000|The zebra crossing is on Main Street.\nu|1793527260000|Thanks, close it\n",
    )
    .unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let cfg = config(enabled, &claude, &fake);
    let db = Db::open(&paths.db_path()).unwrap();
    World {
        _tmp: tmp,
        claude,
        fake,
        paths,
        cfg,
        db,
    }
}

fn app(w: &World) -> Router {
    router(new_state(
        w.paths.clone(),
        w.cfg.clone(),
        TOKEN.into(),
        7420,
    ))
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:7420")
        .header("x-eye-token", TOKEN)
        .header("origin", "http://127.0.0.1:7420")
        .header("content-type", "application/json")
        .body(Body::from(body.map(|b| b.to_string()).unwrap_or_default()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn cards(board: &Value) -> Vec<Value> {
    board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|c| c["cards"].as_array().unwrap().clone())
        .collect()
}

#[tokio::test]
async fn two_agents_work_side_by_side() {
    let mut w = world(r#"["claude", "fake"]"#);
    let st = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(st.files_seen, 2);
    let agent_of = |id: &str| -> String { w.db.session_by_id(id).unwrap().unwrap().agent };
    assert_eq!(agent_of(CLAUDE_ID), "claude");
    assert_eq!(agent_of(FAKE_ID), "fake");
    let fake = w.db.session_by_id(FAKE_ID).unwrap().unwrap();
    assert_eq!(
        (fake.prompt_count, fake.reply_count, fake.cwd.as_deref()),
        (2, 1, Some("/work/fake"))
    );
    assert_eq!(fake.title.as_deref(), Some("Find the zebra crossing"));

    assert_eq!(
        agent_eye::agents::enabled(&w.cfg).names_or(),
        "Claude Code or Fake Agent"
    );
    let app = app(&w);
    // the agents
    let (_, a) = call(&app, "GET", "/api/agents", None).await;
    assert_eq!(
        a,
        json!({"agents": [
            {"id": "claude", "name": "Claude Code", "short": "Claude", "default": true},
            {"id": "fake", "name": "Fake Agent", "short": "Fakebot", "default": false},
        ]})
    );
    // the board: both sessions, each says whose it is
    let (_, board) = call(&app, "GET", "/api/board", None).await;
    let all = cards(&board);
    assert_eq!(all.len(), 2);
    let card = |id: &str| all.iter().find(|c| c["id"] == id).unwrap().clone();
    assert_eq!(card(CLAUDE_ID)["agent"], "claude");
    assert_eq!(card(CLAUDE_ID)["agent_name"], "Claude");
    assert_eq!(card(FAKE_ID)["agent"], "fake");
    assert_eq!(card(FAKE_ID)["agent_name"], "Fakebot");
    assert_eq!(
        card(FAKE_ID)["resume_cmd"],
        format!("cd '/work/fake' && fakebot continue {FAKE_ID}")
    );
    assert_eq!(
        card(CLAUDE_ID)["resume_cmd"],
        format!("cd '/work/claude' && claude --resume {CLAUDE_ID}")
    );
    // search finds each agent's own words
    let (_, hit) = call(&app, "GET", "/api/search?q=zebra", None).await;
    assert_eq!(hit["results"][0]["card"]["id"], FAKE_ID);
    assert_eq!(hit["results"][0]["card"]["agent_name"], "Fakebot");
    let (_, hit) = call(&app, "GET", "/api/search?q=platypus", None).await;
    assert_eq!(hit["results"][0]["card"]["id"], CLAUDE_ID);
    // resume: each session gets its own agent's command; the fake agent cannot fork
    let (st, r) = call(
        &app,
        "POST",
        &format!("/api/cards/{FAKE_ID}/resume"),
        Some(json!({"dry_run": true})),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        r["command"],
        format!("cd '/work/fake' && fakebot continue {FAKE_ID}")
    );
    let (st, r) = call(
        &app,
        "POST",
        &format!("/api/cards/{FAKE_ID}/resume"),
        Some(json!({"dry_run": true, "fork": true})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(r["error"].as_str().unwrap().contains("cannot fork"), "{r}");
    let (_, r) = call(
        &app,
        "POST",
        &format!("/api/cards/{CLAUDE_ID}/resume"),
        Some(json!({"dry_run": true, "fork": true})),
    )
    .await;
    assert!(r["command"].as_str().unwrap().ends_with("--fork-session"));
    // conversations are read by each session's own agent
    let (_, c) = call(
        &app,
        "GET",
        &format!("/api/cards/{FAKE_ID}/conversation"),
        None,
    )
    .await;
    assert_eq!(
        c["messages"][1]["text"],
        "The zebra crossing is on Main Street."
    );
    let (_, c) = call(
        &app,
        "GET",
        &format!("/api/cards/{CLAUDE_ID}/conversation"),
        None,
    )
    .await;
    assert_eq!(c["messages"][0]["role"], "user");
    // the archive: Claude Code keeps the layout of before, another agent gets its own folder
    let enabled = agent_eye::agents::enabled(&w.cfg);
    for id in [CLAUDE_ID, FAKE_ID] {
        agent_eye::archive::archive_session(&w.db, &w.paths, &enabled, id).unwrap();
    }
    let archive = w.paths.home.join("archive");
    assert!(archive
        .join(format!("-work-claude/{CLAUDE_ID}.jsonl.zst"))
        .is_file());
    assert!(archive
        .join(format!("@fake/work-fake/{FAKE_ID}.jsonl.zst"))
        .is_file());
    // ideas: pick the agent per idea; the first agent is the default
    let (st, idea) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Try the bot", "folder": "/work/fake", "prompt": "go", "agent": "fake"})),
    )
    .await;
    assert_eq!(
        (st, &idea["agent"], &idea["agent_name"]),
        (StatusCode::OK, &json!("fake"), &json!("Fakebot"))
    );
    let (_, start) = call(
        &app,
        "POST",
        &format!("/api/ideas/{}/start", idea["idea_id"]),
        Some(json!({"dry_run": true})),
    )
    .await;
    assert_eq!(
        start["command"],
        "cd '/work/fake' && fakebot new 'Try the bot'"
    );
    assert!(
        start["session_id"].is_null(),
        "this agent cannot pick the id"
    );
    let (_, plain) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Default agent", "folder": "/work/x"})),
    )
    .await;
    assert!(plain["agent"].is_null());
    assert_eq!(plain["agent_name"], "Claude");
    let (st, bad) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Nope", "agent": "nobody"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(bad["error"].as_str().unwrap().contains("not enabled"));
    let (_, ideas) = call(&app, "GET", "/api/ideas", None).await;
    assert_eq!(ideas["ideas"].as_array().unwrap().len(), 2);
    assert_eq!(ideas["ideas"][0]["agent"], "fake");
}

#[tokio::test]
async fn what_to_show_and_what_to_store_is_each_agents_call() {
    const ID2: &str = "fake-session-0002";
    let mut w = world(r#"["claude", "fake"]"#);
    std::fs::write(
        w.fake.join(format!("logs/work-fake/{ID2}.log")),
        format!(
            "cwd|/work/fake\nu|1793530000000|!reset\nu|1793530005000|/model\nu|1793530010000|Check the giraffe\nr|1793530020000|Giraffe checked.{}\n",
            fake_agent::RECAP_FOOTER
        ),
    )
    .unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    let recap = |w: &World| w.db.session_by_id(ID2).unwrap().unwrap().recap;
    assert_eq!(recap(&w).as_deref(), Some("Giraffe checked."));

    // a recap stored before the scanner cleaned them is cleaned once, by the agent that owns it
    w.db.conn
        .execute(
            "UPDATE session SET recap=?1 WHERE id=?2",
            [&format!("Old recap{}", fake_agent::RECAP_FOOTER), ID2],
        )
        .unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(
        recap(&w).as_deref(),
        Some(format!("Old recap{}", fake_agent::RECAP_FOOTER).as_str()),
        "once per agent: a later scan leaves it alone"
    );
    w.db.conn
        .execute("DELETE FROM meta WHERE key='recaps_cleaned:fake'", [])
        .unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(recap(&w).as_deref(), Some("Old recap"));

    // the detail shows the prompts the agent says are prompts: "!reset" is the fake agent's
    // built-in command, "/model" is Claude Code's and plain text here
    let app = app(&w);
    let (_, d) = call(&app, "GET", &format!("/api/cards/{ID2}"), None).await;
    assert_eq!(d["prompts"], json!(["/model", "Check the giraffe"]));
    assert_eq!(d["first_prompt"], "/model");
    let (_, d) = call(&app, "GET", &format!("/api/cards/{FAKE_ID}"), None).await;
    assert_eq!(
        d["prompts"],
        json!(["Find the zebra crossing", "Thanks, close it"])
    );
}

#[tokio::test]
async fn hooks_of_each_agent_reach_their_own_sessions() {
    use agent_eye::agents::{self, Agents};
    let mut w = world(r#"["claude", "fake"]"#);
    scan(&mut w.db, &w.cfg, false).unwrap();
    let enabled: Agents = agents::enabled(&w.cfg);
    // an event of Claude Code makes its session wait; the same text for another agent is ignored
    // because that adapter does not interpret hook events
    let q = agent_eye::hook::queue_path(&w.paths);
    std::fs::create_dir_all(q.parent().unwrap()).unwrap();
    let lines = [
        json!({"ts": 5, "event": "Stop", "session_id": CLAUDE_ID, "last_message_tail": "Shall I go on?"}),
        json!({"ts": 6, "event": "Stop", "session_id": FAKE_ID, "agent": "fake", "last_message_tail": "Shall I go on?"}),
    ];
    std::fs::write(
        &q,
        lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
    )
    .unwrap();
    assert_eq!(
        agent_eye::daemon::drain_queue(&w.db, &w.paths, &enabled).unwrap(),
        2
    );
    let waits = |id: &str| w.db.session_by_id(id).unwrap().unwrap().waiting_since;
    assert_eq!(
        waits(CLAUDE_ID),
        Some(5),
        "no agent field means Claude Code"
    );
    assert_eq!(waits(FAKE_ID), None);
}

/// Claude Code's own live status says `waiting` during a prompt, so the hooks need not and do not
/// override it: once you approve and it is busy again, busy wins while the wait is still recorded.
#[tokio::test]
async fn claude_busy_after_an_approval_is_running_though_the_wait_is_still_recorded() {
    use agent_eye::agents;
    let mut w = world(r#"["claude"]"#);
    let enabled = agents::enabled(&w.cfg);
    let pid = std::process::id();
    let sessions = w.claude.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let live = |status: &str, at: i64| {
        std::fs::write(
            sessions.join(format!("{pid}.json")),
            format!(
                r#"{{"pid":{pid},"sessionId":"{CLAUDE_ID}","cwd":"/work/claude","status":"{status}","entrypoint":"cli","updatedAt":{at}}}"#
            ),
        )
        .unwrap();
    };
    let tick = |w: &mut World, event: &str, payload: Value| {
        agent_eye::hook::run(
            &w.paths,
            enabled[0].as_ref(),
            event,
            payload.to_string().as_bytes(),
        );
        agent_eye::daemon::drain_queue(&w.db, &w.paths, &enabled).unwrap();
        scan(&mut w.db, &w.cfg, false).unwrap();
    };
    let card = |w: &World| {
        let s = w.db.session_by_id(CLAUDE_ID).unwrap().unwrap();
        let input = agent_eye::columns::Input::from_session(&s, false, s.last_activity_at.unwrap());
        (
            s.live_status.clone(),
            agent_eye::columns::compute(&input).as_str(),
            s.blocked(),
        )
    };
    live("busy", 1);
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(card(&w), (Some("busy".into()), "running", false));
    // the prompt: Claude Code reports waiting itself
    tick(
        &mut w,
        "PermissionRequest",
        json!({"session_id": CLAUDE_ID, "tool_name": "Bash", "tool_input": {"command": "git push"}}),
    );
    live("waiting", 2);
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(card(&w), (Some("waiting".into()), "needs_you", true));
    // you approve: busy again, and no hook says the prompt was answered
    live("busy", 3);
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!(card(&w), (Some("busy".into()), "running", false));
    let s = w.db.session_by_id(CLAUDE_ID).unwrap().unwrap();
    assert!(
        s.waiting_since.is_some() && s.waiting_prompt,
        "still recorded"
    );
    // PostToolUse is how eye sees a Codex prompt answered; Claude Code's adapter ignores it
    tick(
        &mut w,
        "PostToolUse",
        json!({"session_id": CLAUDE_ID, "tool_name": "Bash"}),
    );
    let s = w.db.session_by_id(CLAUDE_ID).unwrap().unwrap();
    assert!(s.waiting_since.is_some() && s.waiting_prompt);
    assert_eq!(card(&w), (Some("busy".into()), "running", false));
    // the reply ends and clears the wait
    tick(
        &mut w,
        "Stop",
        json!({"session_id": CLAUDE_ID, "last_assistant_message": "Pushed."}),
    );
    assert!(w
        .db
        .session_by_id(CLAUDE_ID)
        .unwrap()
        .unwrap()
        .waiting_since
        .is_none());
}

/// One prompt and no outputs is archive material once the session is closed; while it is open it
/// is somebody's work in progress.
#[tokio::test]
async fn an_open_claude_session_with_one_prompt_is_paused_not_archived() {
    let mut w = world(r#"["claude"]"#);
    std::fs::write(
        w.claude
            .join(format!("projects/-work-claude/{CLAUDE_ID}.jsonl")),
        format!(
            "{}\n{}\n",
            json!({"type":"user","timestamp":"2026-11-01T10:00:00Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"content":"Investigate the platypus report"}}),
            json!({"type":"assistant","timestamp":"2026-11-01T10:00:05Z","cwd":"/work/claude","sessionId":CLAUDE_ID,
                   "message":{"id":"m1","content":[{"type":"text","text":"The platypus report is fine."}]}}),
        ),
    )
    .unwrap();
    let app = app(&w);
    scan(&mut w.db, &w.cfg, false).unwrap();
    let (_, c) = call(&app, "GET", &format!("/api/cards/{CLAUDE_ID}"), None).await;
    assert_eq!(c["column"], "archive", "closed and trivial");

    let pid = std::process::id();
    let sessions = w.claude.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join(format!("{pid}.json")),
        format!(
            r#"{{"pid":{pid},"sessionId":"{CLAUDE_ID}","cwd":"/work/claude","status":"idle","entrypoint":"cli","updatedAt":1}}"#
        ),
    )
    .unwrap();
    scan(&mut w.db, &w.cfg, false).unwrap();
    let (_, c) = call(&app, "GET", &format!("/api/cards/{CLAUDE_ID}"), None).await;
    assert_eq!(c["column"], "paused", "open now");
}

#[tokio::test]
async fn claude_can_be_switched_off() {
    // sessions Claude Code recorded while it was on stay in the database
    let mut w = world(r#"["claude", "fake"]"#);
    scan(&mut w.db, &w.cfg, false).unwrap();
    assert!(w.db.session_by_id(CLAUDE_ID).unwrap().is_some());
    // scanning with Claude off reads only the fake agent and leaves Claude's rows alone
    w.enable(r#"["fake"]"#);
    scan(&mut w.db, &w.cfg, true).unwrap();
    assert!(w.db.session_by_id(CLAUDE_ID).unwrap().is_some());
    assert_eq!(
        w.db.session_by_id(CLAUDE_ID)
            .unwrap()
            .unwrap()
            .transcript_state
            .as_deref(),
        Some("present")
    );
    let app = app(&w);
    let (_, a) = call(&app, "GET", "/api/agents", None).await;
    assert_eq!(a["agents"].as_array().unwrap().len(), 1);
    assert_eq!(a["agents"][0]["id"], "fake");
    assert_eq!(a["agents"][0]["default"], true);
    let (_, board) = call(&app, "GET", "/api/board", None).await;
    let ids: Vec<String> = cards(&board)
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ids,
        [FAKE_ID],
        "Claude sessions are not shown while it is off"
    );
    let (_, hit) = call(&app, "GET", "/api/search?q=platypus", None).await;
    assert!(hit["results"].as_array().unwrap().is_empty());
    // an idea for the default agent now starts in the fake one
    let (_, idea) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Whichever", "folder": "/work/x", "prompt": "go"})),
    )
    .await;
    let (_, start) = call(
        &app,
        "POST",
        &format!("/api/ideas/{}/start", idea["idea_id"]),
        Some(json!({"dry_run": true})),
    )
    .await;
    assert!(start["command"].as_str().unwrap().contains("fakebot"));
    // and Claude is not accepted as an idea's agent
    let (st, _) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Nope", "agent": "claude"})),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    // turning it on again brings its sessions back
    w.enable(r#"["claude", "fake"]"#);
    let again = router(new_state(
        w.paths.clone(),
        w.cfg.clone(),
        TOKEN.into(),
        7420,
    ));
    let (_, board) = call(&again, "GET", "/api/board", None).await;
    let sessions = cards(&board)
        .into_iter()
        .filter(|c| c["kind"] != "idea")
        .count();
    assert_eq!(sessions, 2);
}

#[tokio::test]
async fn no_agent_at_all_is_an_empty_board_not_a_crash() {
    let mut w = world("[]");
    let st = scan(&mut w.db, &w.cfg, false).unwrap();
    assert_eq!((st.files_seen, st.files_parsed), (0, 0));
    let app = app(&w);
    let (s, a) = call(&app, "GET", "/api/agents", None).await;
    assert_eq!((s, a), (StatusCode::OK, json!({"agents": []})));
    let (s, board) = call(&app, "GET", "/api/board", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(cards(&board).is_empty());
    let (s, found) = call(&app, "GET", "/api/search?q=anything", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(found["results"].as_array().unwrap().is_empty());
    // an idea can be parked; starting it says there is no agent
    let (s, idea) = call(
        &app,
        "POST",
        "/api/ideas",
        Some(json!({"title": "Later", "folder": "/work/x"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert!(idea["agent_name"].is_null());
    let (s, e) = call(
        &app,
        "POST",
        &format!("/api/ideas/{}/start", idea["idea_id"]),
        Some(json!({"dry_run": true})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(e["error"].as_str().unwrap().contains("no agent is enabled"));
}
