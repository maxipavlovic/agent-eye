//! Background tasks, loops, why a session waits and its remote control link, from the files of
//! Claude Code and Codex through the scanner to the card the board shows.

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::server::{new_state, router};
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
const CLAUDE_SESSION: &str = "dddddddd-0000-4000-8000-0000000000a1";
const CODEX_SESSION: &str = "00000000-0000-4000-8000-000000000001";
const CODEX_FIXTURE: &str = include_str!("fixtures/codex-background-process.jsonl");

struct World {
    _tmp: tempfile::TempDir,
    agent_dir: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl World {
    fn new(agent: &str) -> World {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join(agent);
        std::fs::create_dir_all(&agent_dir).unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"{agent}\"]\n[agents.{agent}]\ndir = \"{}\"\n",
            agent_dir.display()
        ))
        .unwrap();
        let db = Db::open(&paths.db_path()).unwrap();
        World {
            _tmp: tmp,
            agent_dir,
            paths,
            cfg,
            db,
        }
    }

    fn scan(&mut self) {
        scan(&mut self.db, &self.cfg, false).unwrap();
    }

    fn card(&self, id: &str) -> Value {
        let app: Router = router(new_state(
            self.paths.clone(),
            self.cfg.clone(),
            TOKEN.into(),
            7420,
        ));
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let req = Request::builder()
                .method("GET")
                .uri(format!("/api/cards/{id}"))
                .header("host", "127.0.0.1:7420")
                .header("x-eye-token", TOKEN)
                .header("origin", "http://127.0.0.1:7420")
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice(&bytes).unwrap()
        })
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

// ---------- Claude Code ----------

fn line(ty: &str, ts: &str, message: Value, extra: Value) -> String {
    let mut v = json!({"type": ty, "timestamp": ts, "cwd": "/work/app", "sessionId": CLAUDE_SESSION,
        "message": message});
    for (k, x) in extra.as_object().into_iter().flatten() {
        v[k] = x.clone();
    }
    format!("{v}\n")
}

fn text(role: &str, ts: &str, t: &str) -> String {
    let content = if role == "user" {
        json!(t)
    } else {
        json!([{"type": "text", "text": t}])
    };
    line(
        role,
        ts,
        json!({"role": role, "content": content}),
        json!({}),
    )
}

fn tool_use(ts: &str, id: &str, name: &str, input: Value) -> String {
    line(
        "assistant",
        ts,
        json!({"role": "assistant", "content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}),
        json!({}),
    )
}

fn tool_result(ts: &str, id: &str, t: &str, own: Value) -> String {
    line(
        "user",
        ts,
        json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": id, "content": t}]}),
        json!({"toolUseResult": own}),
    )
}

fn notice(ts: &str, id: &str) -> String {
    json!({"type": "queue-operation", "operation": "enqueue", "timestamp": ts,
        "sessionId": CLAUDE_SESSION,
        "content": format!("<task-notification>\n<task-id>{id}</task-id>\n<status>completed</status>\n</task-notification>")})
    .to_string()
        + "\n"
}

fn claude_world(live: Value) -> (World, PathBuf) {
    let w = World::new("claude");
    let proj = w.agent_dir.join("projects/-work-app");
    std::fs::create_dir_all(&proj).unwrap();
    let file = proj.join(format!("{CLAUDE_SESSION}.jsonl"));
    let now = agent_eye::util::now_ms();
    let ts = |ago_min: i64| iso(now - ago_min * 60_000);
    let mut body = text("user", &ts(30), "Start the dev server and keep it up");
    body += &text("assistant", &ts(29), "Starting it now.");
    body += &text("user", &ts(28), "And the tests");
    body += &tool_use(
        &ts(10),
        "tu1",
        "Bash",
        json!({"command": "npm run dev", "run_in_background": true}),
    );
    body += &tool_result(
        &ts(10),
        "tu1",
        "Command running in background with ID: bsrv1",
        json!({"backgroundTaskId": "bsrv1"}),
    );
    body += &text("assistant", &ts(9), "The server is up.");
    std::fs::write(&file, body).unwrap();
    let mut session = json!({"pid": std::process::id(), "sessionId": CLAUDE_SESSION,
        "cwd": "/work/app", "status": "idle", "updatedAt": now});
    for (k, x) in live.as_object().into_iter().flatten() {
        session[k] = x.clone();
    }
    std::fs::create_dir_all(w.agent_dir.join("sessions")).unwrap();
    std::fs::write(
        w.agent_dir
            .join(format!("sessions/{}.json", std::process::id())),
        session.to_string(),
    )
    .unwrap();
    (w, file)
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` of a Unix ms time.
fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[test]
fn an_idle_session_with_a_background_server_is_running_and_says_what() {
    let (mut w, file) = claude_world(json!({"bridgeSessionId": "session_01AbC"}));
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(c["column"], "running");
    assert_eq!(c["live_status"], "idle");
    assert_eq!(c["background"][0]["label"], "npm run dev");
    assert_eq!(c["background"][0]["kind"], "shell");
    assert!(c["activity"]
        .as_str()
        .unwrap()
        .starts_with("background: npm run dev · "));
    assert_eq!(c["remote_url"], "https://claude.ai/code/session_01AbC");
    assert_eq!(c["loops"], json!([]));

    // the notice arrives in a later read: the task ends and the session rests
    let now = agent_eye::util::now_ms();
    append(&file, &notice(&iso(now), "bsrv1"));
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(c["column"], "paused");
    assert_eq!(c["background"], json!([]));
    assert_eq!(c["activity"], Value::Null);
}

#[test]
fn a_loop_keeps_an_open_session_in_running_until_it_is_deleted() {
    let (mut w, file) = claude_world(json!({}));
    let now = agent_eye::util::now_ms();
    let ts = iso(now);
    append(&file, &notice(&ts, "bsrv1"));
    append(
        &file,
        &tool_use(
            &ts,
            "tu2",
            "CronCreate",
            json!({"cron": "*/10 * * * *", "prompt": "check the build", "recurring": true}),
        ),
    );
    append(
        &file,
        &tool_result(&ts, "tu2", "scheduled", json!({"id": "job1"})),
    );
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(c["column"], "running");
    assert_eq!(c["loops"][0]["schedule"], "every 10 min");
    assert!(c["loops"][0]["next_at"].as_i64().unwrap() > now);
    assert!(c["activity"]
        .as_str()
        .unwrap()
        .starts_with("↻ every 10 min · next "));
    append(
        &file,
        &tool_use(&ts, "tu3", "CronDelete", json!({"id": "job1"})),
    );
    w.scan();
    assert_eq!(w.card(CLAUDE_SESSION)["column"], "paused");
}

#[test]
fn a_session_that_waits_on_a_permission_prompt_needs_you_and_says_why() {
    let (mut w, _) = claude_world(json!({"status": "waiting", "waitingFor": "permission prompt"}));
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(c["column"], "needs_you");
    assert_eq!(c["next"], "You: Permission prompt");
    // the background server does not hide it, and a dialog the user opened is no ask
    let (mut w, _) = claude_world(json!({"status": "waiting", "waitingFor": "dialog open"}));
    w.scan();
    assert_eq!(w.card(CLAUDE_SESSION)["column"], "running");
}

#[test]
fn a_session_whose_process_is_gone_shows_no_background_work() {
    let (mut w, _) =
        claude_world(json!({"pid": 2_000_000_000, "bridgeSessionId": "session_01AbC"}));
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(c["live_status"], Value::Null);
    assert_eq!(c["background"], json!([]));
    assert_eq!(c["remote_url"], Value::Null);
    assert_ne!(c["column"], "running");
    // it is still stored: a session that comes back shows it again
    let n: i64 =
        w.db.conn
            .query_row(
                "SELECT COUNT(*) FROM session WHERE signals IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_session_with_nothing_going_on_has_empty_fields() {
    let w = World::new("claude");
    let proj = w.agent_dir.join("projects/-work-app");
    std::fs::create_dir_all(&proj).unwrap();
    let now = agent_eye::util::now_ms();
    std::fs::write(
        proj.join(format!("{CLAUDE_SESSION}.jsonl")),
        text("user", &iso(now), "hello") + &text("assistant", &iso(now), "hi"),
    )
    .unwrap();
    let mut w = w;
    w.scan();
    let c = w.card(CLAUDE_SESSION);
    assert_eq!(
        (
            &c["activity"],
            &c["background"],
            &c["loops"],
            &c["remote_url"]
        ),
        (&Value::Null, &json!([]), &json!([]), &Value::Null)
    );
    let n: i64 =
        w.db.conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key LIKE 'pending:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(n, 0, "nothing to carry, nothing stored");
}

// ---------- Codex ----------

fn codex_world(upto_line: usize) -> (World, PathBuf, Vec<std::fs::File>) {
    let w = World::new("codex");
    let dir = w.agent_dir.join("sessions/2026/11/11");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("rollout-2026-11-11T20-00-29-{CODEX_SESSION}.jsonl"));
    let head: String = CODEX_FIXTURE
        // the recorded run was a `codex exec`, which eye leaves out; make it an interactive one
        .replace("<prompt>", "Run two sleeps in the background and report")
        .replace("codex_exec", "codex_cli_rs")
        .replace("\"source\":\"exec\"", "\"source\":\"cli\"")
        .lines()
        .take(upto_line)
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&file, head).unwrap();
    let locks = w.agent_dir.join("thread-writer-locks");
    std::fs::create_dir_all(&locks).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(locks.join(format!("{CODEX_SESSION}.lock")))
        .unwrap();
    held.try_lock().unwrap();
    (w, file, vec![held])
}

#[test]
fn a_codex_background_shell_keeps_the_session_in_running_until_its_item_closes() {
    // the fixture up to the first result: `sleep 80` was started and left running
    let (mut w, file, _lock) = codex_world(5);
    w.scan();
    let c = w.card(CODEX_SESSION);
    assert_eq!(c["live_status"], "busy");
    assert_eq!(c["background"][0]["label"], "sleep 80");
    assert_eq!(c["background"][0]["id"], "36096");
    // the rest of the run: a foreground command, the turn's end, and the item that closes the shell
    for l in CODEX_FIXTURE.lines().skip(5) {
        append(&file, &format!("{l}\n"));
    }
    w.scan();
    let c = w.card(CODEX_SESSION);
    assert_eq!(c["background"], json!([]));
    assert_eq!(c["live_status"], "idle");
}

#[test]
fn a_codex_idle_session_with_a_background_shell_is_running() {
    // up to the turn's end, but before the shell's closing item
    let (mut w, _, _lock) = codex_world(10);
    w.scan();
    let c = w.card(CODEX_SESSION);
    assert_eq!(c["live_status"], "idle");
    assert_eq!(c["column"], "running");
    assert_eq!(c["background"].as_array().unwrap().len(), 1);
}

#[test]
fn an_active_codex_goal_is_a_loop_and_ends_with_it() {
    let (mut w, _, _lock) = codex_world(4);
    let db = rusqlite::Connection::open(w.agent_dir.join("goals_1.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE thread_goals(thread_id TEXT PRIMARY KEY, goal_id TEXT, objective TEXT, status TEXT,
           token_budget INTEGER, tokens_used INTEGER, time_used_seconds INTEGER, created_at_ms INTEGER, updated_at_ms INTEGER);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_goals(thread_id, goal_id, objective, status, created_at_ms) VALUES(?1, 'g', 'ship the release', 'active', 1)",
        [CODEX_SESSION],
    )
    .unwrap();
    w.scan();
    let c = w.card(CODEX_SESSION);
    assert_eq!(c["loops"][0]["kind"], "goal");
    assert_eq!(c["activity"], "↻ goal: ship the release");
    assert_eq!(c["remote_url"], Value::Null);
    db.execute("UPDATE thread_goals SET status='complete'", [])
        .unwrap();
    w.scan();
    assert_eq!(w.card(CODEX_SESSION)["loops"], json!([]));
}

#[test]
fn a_codex_home_without_goals_or_background_work_is_unchanged() {
    let (mut w, _, _lock) = codex_world(4);
    w.scan();
    let c = w.card(CODEX_SESSION);
    assert_eq!(
        (&c["activity"], &c["background"], &c["loops"]),
        (&Value::Null, &json!([]), &json!([]))
    );
}
