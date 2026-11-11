//! The folder side of the idea routes: the card fields, the folder check and the start codes.

use super::*;
use crate::server::test_support::*;
use axum::http::StatusCode;

async fn idea_in(e: &Env, folder: &std::path::Path) -> serde_json::Value {
    let body = json!({"title": "Try it", "folder": folder.to_string_lossy()});
    let (st, idea) = call(e, req("POST", "/api/ideas", Some(body))).await;
    assert_eq!(st, StatusCode::OK);
    idea
}

fn root(e: &Env) -> std::path::PathBuf {
    e._tmp.path().join("root")
}

#[tokio::test]
async fn an_idea_card_says_what_its_folder_is() {
    let e = env_rooted();
    let there = idea_in(&e, &root(&e)).await;
    assert_eq!(there["folder_state"], "ok");
    assert_eq!(there["folder_note"], json!(null));
    let missing = idea_in(&e, &root(&e).join("new")).await;
    assert_eq!(missing["folder_state"], "missing");
    assert_eq!(missing["folder_creatable"], true);
    assert!(missing["folder_note"]
        .as_str()
        .unwrap()
        .contains("Start will create it"));
    let file = root(&e).join("file");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(idea_in(&e, &file).await["folder_state"], "not_a_folder");
    let outside = idea_in(&e, std::path::Path::new("/nonexistent-eye-test/x")).await;
    assert_eq!(outside["folder_state"], "missing");
    assert_eq!(outside["folder_creatable"], false);
    // the board and the detail carry the same fields
    let (_, board) = call(&e, req("GET", "/api/board", None)).await;
    let ideas = board["columns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "ideas")
        .unwrap();
    assert_eq!(ideas["cards"][1]["folder_state"], "missing");
    let (_, d) = call(&e, req("GET", "/api/cards/idea:2", None)).await;
    assert_eq!(d["folder_state"], "missing");
}

#[tokio::test]
async fn the_folder_check_answers_for_a_typed_path() {
    let e = env_rooted();
    let q = |p: &std::path::Path| format!("/api/folder?path={}", p.to_string_lossy());
    let (st, ok) = call(&e, req("GET", &q(&root(&e)), None)).await;
    assert_eq!(
        (st, ok["state"].as_str(), ok["note"].is_null()),
        (StatusCode::OK, Some("ok"), true)
    );
    let (_, m) = call(&e, req("GET", &q(&root(&e).join("a/b")), None)).await;
    assert_eq!(
        (m["state"].as_str(), m["creatable"].as_bool()),
        (Some("missing"), Some(true))
    );
    let (_, u) = call(&e, req("GET", "/api/folder?path=relative", None)).await;
    assert_eq!(u["state"], "unusable");
    // nothing was created
    assert!(!root(&e).join("a").exists());
    // the same guard as every other route
    let mut bad = req("GET", &q(&root(&e)), None);
    bad.headers_mut().remove("x-eye-token");
    assert_eq!(call(&e, bad).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_start_in_a_missing_folder_answers_folder_missing() {
    let e = env_rooted();
    let dir = root(&e).join("new");
    idea_in(&e, &dir).await;
    for body in [
        json!({}),
        json!({"dry_run": false}),
        json!({"dry_run": true}),
    ] {
        let (st, r) = call(&e, req("POST", "/api/ideas/1/start", Some(body))).await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(r["code"], "folder_missing");
        assert!(r["error"].as_str().unwrap().contains("does not exist"));
    }
    assert!(!dir.exists());
}

#[tokio::test]
async fn a_dry_run_with_create_folder_creates_nothing() {
    let e = env_rooted();
    let dir = root(&e).join("new");
    idea_in(&e, &dir).await;
    let body = json!({"dry_run": true, "create_folder": true});
    let (st, r) = call(&e, req("POST", "/api/ideas/1/start", Some(body))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["creates_folder"], true);
    assert!(r["command"].as_str().unwrap().starts_with("cd '"));
    assert!(!dir.exists());
}

#[tokio::test]
async fn a_folder_that_cannot_be_made_is_refused_with_a_reason() {
    let e = env_rooted();
    idea_in(&e, std::path::Path::new("/nonexistent-eye-test/x")).await;
    let body = json!({"dry_run": true, "create_folder": true});
    let (st, r) = call(&e, req("POST", "/api/ideas/1/start", Some(body))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(r["error"]
        .as_str()
        .unwrap()
        .contains("inside your home folder"));
    assert!(r["code"].is_null());
    let file = root(&e).join("file");
    std::fs::write(&file, "x").unwrap();
    idea_in(&e, &file).await;
    let (st, r) = call(
        &e,
        req(
            "POST",
            "/api/ideas/2/start",
            Some(json!({"create_folder": true})),
        ),
    )
    .await;
    assert_eq!(
        (st, r["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("Not a folder"))
    );
}

#[tokio::test]
async fn an_existing_folder_starts_as_before() {
    let e = env_rooted();
    idea_in(&e, &root(&e)).await;
    let (st, r) = call(
        &e,
        req("POST", "/api/ideas/1/start", Some(json!({"dry_run": true}))),
    )
    .await;
    assert_eq!(
        (st, r["creates_folder"].as_bool()),
        (StatusCode::OK, Some(false))
    );
}
