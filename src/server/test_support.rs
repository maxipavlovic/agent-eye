//! Helpers the server tests share: an app on a temp home, a request builder and a few rows.

use super::{new_state, new_state_with_face, router};
use crate::config::Config;
use crate::db::Db;
use crate::paths::Paths;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub(super) const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

pub(super) struct Env {
    pub(super) _tmp: tempfile::TempDir,
    pub(super) app: Router,
    pub(super) db: Db,
}

pub(super) fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    let cfg = Config::with_claude_dir(&tmp.path().join("claude"));
    let st = new_state(paths, cfg, TOKEN.into(), 7420);
    Env {
        _tmp: tmp,
        app: router(st),
        db,
    }
}

pub(super) fn env_face(dir: Option<std::path::PathBuf>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    let cfg = Config::with_claude_dir(&tmp.path().join("claude"));
    let st = new_state_with_face(paths, cfg, TOKEN.into(), 7420, dir);
    Env {
        _tmp: tmp,
        app: router(st),
        db,
    }
}

pub(super) async fn raw(e: &Env, r: Request<Body>) -> (StatusCode, HeaderMap, String) {
    let resp = e.app.clone().oneshot(r).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

pub(super) fn face_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("assets")).unwrap();
    std::fs::write(
        d.path().join("index.html"),
        "<!doctype html><title>face</title>",
    )
    .unwrap();
    std::fs::write(d.path().join("assets/app-abc123.js"), "console.log(1)").unwrap();
    std::fs::write(d.path().join("assets/app-abc123.css"), "a{}").unwrap();
    std::fs::write(d.path().join("secret.txt"), "outside").unwrap();
    d
}

pub(super) fn req(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:7420")
        .header("x-eye-token", TOKEN)
        .header("origin", "http://127.0.0.1:7420");
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

pub(super) async fn call(e: &Env, r: Request<Body>) -> (StatusCode, Value) {
    let resp = e.app.clone().oneshot(r).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub(super) fn add_session(db: &Db, id: &str, title: &str, ai: Option<&str>, prompts: i64, at: i64) {
    db.conn
        .execute(
            "INSERT INTO session(id,title,cwd,kind,prompt_count,last_activity_at,ai_status,state_line,next_step,git_branch,transcript_state)
             VALUES(?1,?2,'/tmp/demo','interactive',?3,?4,?5,'state here','next here','HEAD','present')",
            rusqlite::params![id, title, prompts, at, ai],
        )
        .unwrap();
}

pub(super) fn add_output(db: &Db, sid: &str, kind: &str, value: &str, url: Option<&str>, at: i64) {
    db.conn
        .execute(
            "INSERT INTO output(session_id,kind,value,url,first_seen_at) VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![sid, kind, value, url, at],
        )
        .unwrap();
}

pub(super) fn env_cfg(mut cfg: Config) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    cfg.override_agent_dir("claude", tmp.path().join("claude"));
    let st = new_state(paths, cfg, TOKEN.into(), 7420);
    Env {
        _tmp: tmp,
        app: router(st),
        db,
    }
}

pub(super) fn card_ids(b: &Value, col: &str) -> Vec<String> {
    let c = b["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == col)
        .unwrap();
    c["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

pub(super) fn write_transcript(e: &Env, id: &str, pairs: usize) {
    let t = e._tmp.path().join(format!("{id}.jsonl"));
    let mut body = String::new();
    for i in 0..pairs {
        body.push_str(&format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-11-08T10:00:00.000Z\",\"message\":{{\"content\":\"question {i}\"}}}}\n{{\"type\":\"assistant\",\"timestamp\":\"2026-11-08T10:00:05.000Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"answer {i}\"}}]}}}}\n"
        ));
    }
    std::fs::write(&t, body).unwrap();
    e.db.conn
        .execute(
            "UPDATE session SET transcript_path=?2 WHERE id=?1",
            rusqlite::params![id, t.to_string_lossy()],
        )
        .unwrap();
}
