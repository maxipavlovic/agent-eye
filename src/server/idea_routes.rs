//! Ideas: list, add, edit, start.

use super::agent_routes::checked_agent;
use super::api::{blocking, open_db, ApiError, ApiResult};
use super::card::idea_card;
use super::Shared;
use crate::hide;
use crate::ideas;
use crate::model::Idea;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

pub(super) async fn api_ideas(State(st): State<Shared>) -> ApiResult {
    let db = open_db(&st)?;
    let enabled = st.cfg.enabled_agent_ids();
    let open: Vec<Idea> = ideas::list_open(&db)?
        .into_iter()
        .filter(|i| !hide::idea_off(&enabled, i))
        .collect();
    Ok(Json(json!({"ideas": open})))
}

#[derive(Deserialize)]
pub(super) struct IdeaBody {
    title: String,
    notes: Option<String>,
    prompt: Option<String>,
    folder: Option<String>,
    agent: Option<String>,
}

pub(super) async fn api_idea_add(State(st): State<Shared>, Json(b): Json<IdeaBody>) -> ApiResult {
    let db = open_db(&st)?;
    let folder = b.folder.as_deref().map(expand_tilde);
    let agent = checked_agent(&st.agents, b.agent.as_deref())?;
    let idea = ideas::add(
        &db,
        &ideas::NewIdea {
            title: &b.title,
            notes: b.notes.as_deref(),
            prompt: b.prompt.as_deref(),
            folder: folder.as_deref(),
            from_session: None,
            agent,
        },
    )
    .map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(Json(idea_card(&idea, &st.agents)))
}

#[derive(Deserialize)]
pub(super) struct IdeaEditBody {
    title: Option<String>,
    notes: Option<String>,
    prompt: Option<String>,
    folder: Option<String>,
    agent: Option<String>,
}

pub(super) async fn api_idea_edit(
    State(st): State<Shared>,
    Path(id): Path<i64>,
    Json(b): Json<IdeaEditBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    ideas::get(&db, id)?.ok_or_else(|| ApiError::not_found("no such idea"))?;
    checked_agent(&st.agents, b.agent.as_deref())?;
    let e = ideas::Edit {
        title: b.title,
        notes: b.notes,
        prompt: b.prompt,
        folder: b.folder.as_deref().map(expand_tilde),
        agent: b.agent,
    };
    let idea = ideas::edit(&db, id, &e).map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(Json(idea_card(&idea, &st.agents)))
}

pub(super) fn expand_tilde(p: &str) -> String {
    crate::config::expand_tilde(p.trim())
        .to_string_lossy()
        .into_owned()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct StartBody {
    background: bool,
    dry_run: Option<bool>,
}

pub(super) async fn api_idea_start(
    State(st): State<Shared>,
    Path(id): Path<i64>,
    body: Option<Json<StartBody>>,
) -> ApiResult {
    let db = open_db(&st)?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let idea = ideas::get(&db, id)?.ok_or_else(|| ApiError::not_found("no such idea"))?;
    let dry = b.dry_run.unwrap_or(true);
    // a start runs the agent (or osascript, or a clipboard tool) and can take a while
    let (idea_id, background) = (idea.id, b.background);
    let r = blocking(move || {
        ideas::start(&db, &st.cfg, &st.paths, idea_id, background, dry, false)
            .map_err(|e| ApiError::bad(format!("{e:#}")))
    })
    .await?;
    Ok(Json(json!({
        "dry_run": r.dry_run, "mode": r.mode.as_str(), "terminal": r.terminal, "command": r.command,
        "applescript": r.applescript, "session_id": r.session_id,
        "job": r.job, "note": r.output,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use crate::util::now_ms;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn ideas_resume_and_search() {
        let e = env();
        let (st, idea) = call(
            &e,
            req(
                "POST",
                "/api/ideas",
                Some(
                    json!({"title": "Try the importer", "notes": "look at it", "folder": "/tmp/x"}),
                ),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(idea["id"], "idea:1");
        // The detail of an idea has the same fields as a session detail.
        let (_, d) = call(&e, req("GET", "/api/cards/idea:1", None)).await;
        assert_eq!(d["prompts"], json!([]));
        for k in ["first_prompt", "last_assistant_text", "recap", "ai_status"] {
            assert!(d[k].is_null(), "{k}");
        }
        assert!(d["resume_mode"].is_string());
        assert_eq!(
            (d["notes"].as_str(), d["prompt"].is_null()),
            (Some("look at it"), true)
        );
        // edit: notes stay yours, the prompt is what a start sends
        let (st, c) = call(
            &e,
            req(
                "POST",
                "/api/ideas/1",
                Some(json!({"prompt": "Compare it with the old importer", "folder": "/tmp/y"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(
            (c["title"].as_str(), c["has_prompt"].as_bool()),
            (Some("Try the importer"), Some(true))
        );
        let (_, d) = call(&e, req("GET", "/api/cards/idea:1", None)).await;
        assert_eq!(
            (
                d["notes"].as_str(),
                d["prompt"].as_str(),
                d["folder"].as_str()
            ),
            (
                Some("look at it"),
                Some("Compare it with the old importer"),
                Some("/tmp/y")
            )
        );
        assert_eq!(
            call(&e, req("POST", "/api/ideas/1", Some(json!({"title": " "}))))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&e, req("POST", "/api/ideas/9", Some(json!({"title": "x"}))))
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        let (st, p) = call(
            &e,
            req(
                "POST",
                "/api/ideas/1/start",
                Some(json!({"background": false, "dry_run": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(p["command"].as_str().unwrap().contains("--session-id"));

        add_session(&e.db, "aaaaaaaa-1", "Catalog migration", None, 3, now_ms());
        crate::db::fts_write(
            &e.db.conn,
            "aaaaaaaa-1",
            "migrate the catalog\n",
            "",
            true,
            1000,
            1000,
        )
        .unwrap();
        let (_, r) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/resume",
                Some(json!({"dry_run": true})),
            ),
        )
        .await;
        assert_eq!(r["command"], "cd '/tmp/demo' && claude --resume aaaaaaaa-1");
        let mode = r["mode"].as_str().unwrap();
        assert!(["iterm", "terminal", "copy"].contains(&mode), "{r}");
        match r["applescript"].as_str() {
            Some(script) => {
                assert_eq!(mode, "iterm");
                assert!(script.contains("iTerm2"));
            }
            None => assert_ne!(mode, "iterm"),
        }
        let (_, s) = call(&e, req("GET", "/api/search?q=catalog", None)).await;
        assert_eq!(s["results"][0]["card"]["id"], "aaaaaaaa-1");
        let (_, full) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(full["prompts"][0], "migrate the catalog");
        assert!(full["idea"].is_null());
        // a session started from an idea shows the idea's notes
        e.db.conn
            .execute("UPDATE session SET idea_id=1 WHERE id='aaaaaaaa-1'", [])
            .unwrap();
        let (_, full) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(full["idea"]["notes"], "look at it");
    }
}
