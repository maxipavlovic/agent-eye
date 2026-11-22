//! Routes on one card: detail, move, hide, resume, conversation, and search.

use super::api::{blocking, hidden_set, open_db, resolve, wants_all, ApiError, ApiResult};
use super::card::{card_json, idea_card, load_card, output_json};
use super::Shared;
use crate::conversation;
use crate::ideas;
use crate::model::Column;
use crate::remind;
use crate::resume;
use crate::search;
use crate::util::{head_chars, now_ms};
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

pub(super) async fn api_card(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let db = open_db(&st)?;
    let now = now_ms();
    if let Some(n) = id.strip_prefix("idea:") {
        let i = n
            .parse::<i64>()
            .ok()
            .and_then(|n| ideas::get(&db, n).ok().flatten())
            .ok_or_else(|| ApiError::not_found("no such idea"))?;
        let mut v = idea_card(&i, &st.agents, &st.cfg, now);
        v["notes"] = json!(i.notes);
        v["prompt"] = json!(i.prompt);
        v["prompts"] = json!([]);
        v["first_prompt"] = Value::Null;
        v["last_assistant_text"] = Value::Null;
        resume_fields(&mut v, &resume::target(&st.cfg, &st.paths));
        return Ok(Json(v));
    }
    let id = resolve(&db, &id)?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let streams = db.streams_for(&id)?;
    let outputs = db.outputs_for(&id)?;
    let mut v = card_json(&s, &streams, &outputs, now, true, st.card_ctx());
    v["outputs"] = json!(outputs.iter().map(output_json).collect::<Vec<_>>());
    v["topics"] = json!(streams
        .iter()
        .filter(|x| x.kind == "topic")
        .map(
            |t| json!({"key": t.key, "title": t.title.as_deref().unwrap_or(&t.key),
                        "status": t.status, "next": t.detail})
        )
        .collect::<Vec<_>>());
    v["topics_more"] = json!(0);
    let left = crate::loose::list(&db, Some(&id), false)?;
    v["loose"] = json!(left.len());
    v["night_run"] = super::night_routes::run_of_session(&db, &id)?;
    v["loose_ends"] = json!(left
        .iter()
        .map(crate::loose::LooseEnd::to_json)
        .collect::<Vec<_>>());
    v["loose_summary"] = json!(crate::loose::summary_text(&left));
    v["worked"] = json!(crate::worked::summary::of_session(
        &db.conn,
        &id,
        &crate::worked::summary::local_offset_ms
    )?);
    v["usage"] = match crate::usage::report::of_session(&db.conn, &st.cfg.usage, &id)? {
        Some(sum) => crate::usage::report::session_json(&sum),
        None => Value::Null,
    };
    // Stored prompt lines include commands and paste tags; the agent says what is worth showing.
    let readable = |l: &str| st.agents.display_prompt(&s, l);
    let lines = db.prompt_lines(&id)?;
    let shown: Vec<String> = lines.iter().filter_map(|l| readable(l)).collect();
    v["prompts"] = json!(shown
        .iter()
        .skip(shown.len().saturating_sub(10))
        .map(|p| head_chars(p, 400))
        .collect::<Vec<_>>());
    // The stored first prompt can be a command ("/model"); then the first readable line wins.
    let first = s
        .first_prompt
        .as_deref()
        .and_then(readable)
        .or_else(|| lines.iter().find_map(|l| readable(l)));
    v["first_prompt"] = json!(first.as_deref().map(|p| head_chars(p, 600)));
    v["last_assistant_text"] = json!(s
        .last_assistant_text
        .as_deref()
        .map(|p| crate::util::tail_chars(p, 600)));
    resume_fields(&mut v, &resume::target(&st.cfg, &st.paths));
    // A session started from an idea keeps the idea's notes in view.
    v["idea"] = match s.idea_id.map(|n| ideas::get(&db, n)).transpose()?.flatten() {
        Some(i) => json!({"id": i.id, "title": i.title, "notes": i.notes, "prompt": i.prompt}),
        None => Value::Null,
    };
    Ok(Json(v))
}

#[derive(Deserialize)]
pub(super) struct MoveBody {
    column: String,
}

pub(super) async fn api_move(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<MoveBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    let want = b.column.trim().to_lowercase();
    if let Some(n) = id.strip_prefix("idea:") {
        let n: i64 = n.parse().map_err(|_| ApiError::bad("bad idea id"))?;
        if Column::parse(&want) == Some(Column::Archive) {
            ideas::archive(&db, n)?;
            return Ok(Json(json!({"ok": true})));
        }
        return Err(ApiError::bad(
            "an idea stays in Ideas until you start it; drop it on Archive to remove it",
        ));
    }
    let id = resolve(&db, &id)?;
    let now = now_ms();
    if want == "auto" {
        db.conn.execute(
            "UPDATE session SET column_manual=NULL, column_manual_at=NULL, archived=0 WHERE id=?1",
            [&id],
        )?;
    } else {
        let col = Column::parse(&want)
            .ok_or_else(|| ApiError::bad(format!("unknown column '{}'", b.column)))?;
        match col {
            Column::Ideas => {
                return Err(ApiError::bad("sessions cannot be moved to Ideas"));
            }
            Column::Archive => {
                db.conn.execute(
                    "UPDATE session SET archived=1, column_manual=NULL, column_manual_at=NULL WHERE id=?1",
                    [&id],
                )?;
            }
            c => {
                db.conn.execute(
                    "UPDATE session SET archived=0, column_manual=?2, column_manual_at=?3 WHERE id=?1",
                    rusqlite::params![id, c.as_str(), now],
                )?;
            }
        }
    }
    // putting it somewhere by hand is acting on a reminder that came due
    let finished = matches!(Column::parse(&want), Some(Column::Done | Column::Archive));
    remind::answered(&db, &id, now, finished)?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    Ok(Json(load_card(&db, &s, now, st.card_ctx())?))
}

#[derive(Deserialize)]
pub(super) struct HiddenBody {
    hidden: bool,
}

/// Same as `eye hide` / `eye unhide`: a manual override that beats the rule.
pub(super) async fn api_hidden(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<HiddenBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    if id.starts_with("idea:") {
        return Err(ApiError::bad(
            "ideas cannot be hidden; archive them instead",
        ));
    }
    let id = resolve(&db, &id)?;
    db.set_hidden(&id, Some(b.hidden))?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    Ok(Json(load_card(&db, &s, now_ms(), st.card_ctx())?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ForgetBody {
    /// The person said yes after the page told them what goes.
    confirm: bool,
    /// Also remove the agent's own copy of the transcript.
    transcript: bool,
}

/// Same as `eye forget <id> [--transcript] --yes`, behind the guard and only with `confirm: true`.
pub(super) async fn api_forget(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<ForgetBody>,
) -> ApiResult {
    use crate::forget::{self, OwnCopy};
    if id.starts_with("idea:") {
        return Err(ApiError::bad(
            "ideas are not forgotten; archive them instead",
        ));
    }
    if !b.confirm {
        return Err(ApiError::bad(
            "forgetting needs the person's confirmation (confirm: true)",
        ));
    }
    blocking(move || {
        let db = open_db(&st)?;
        let id = resolve(&db, &id)?;
        let s = db
            .session_by_id(&id)?
            .ok_or_else(|| ApiError::not_found("no such session"))?;
        if forget::is_open(&s) {
            return Err(ApiError::conflict(
                "open_now",
                "this session is open now: close it first",
            ));
        }
        let done = forget::forget(&db, &st.paths, &st.cfg, &st.agents, &[s], b.transcript)?;
        let o = &done.outcomes[0];
        let (own, detail) = match &o.own {
            OwnCopy::Kept => ("kept", vec![]),
            OwnCopy::Removed(g) => ("removed", g.clone()),
            OwnCopy::Unsupported(w) => ("unsupported", vec![w.clone()]),
            OwnCopy::Failed(e) => ("failed", vec![e.clone()]),
        };
        if !o.forgotten {
            return Err(ApiError::conflict(
                "not_done",
                detail.first().cloned().unwrap_or_default(),
            ));
        }
        Ok(Json(serde_json::json!({
            "ok": true, "id": o.id, "agent_copy": own, "detail": detail,
        })))
    })
    .await
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ResumeBody {
    dry_run: Option<bool>,
    /// Branch a new session from this one (`--fork-session`).
    fork: Option<bool>,
}

/// What Resume will do on this machine, for the card detail: `resume_mode` (`iterm`, `terminal` or
/// `copy`) and `resume_terminal` (the Linux terminal's name, or null).
fn resume_fields(v: &mut Value, target: &resume::Target) {
    v["resume_mode"] = json!(target.mode().as_str());
    v["resume_terminal"] = json!(target.terminal());
}

pub(super) fn plan_json(p: &resume::Plan) -> Value {
    json!({"dry_run": true, "mode": p.mode().as_str(), "terminal": p.target.terminal(),
           "command": p.command, "applescript": p.applescript})
}

pub(super) async fn api_resume(
    State(st): State<Shared>,
    Path(id): Path<String>,
    body: Option<Json<ResumeBody>>,
) -> ApiResult {
    let db = open_db(&st)?;
    let id = resolve(&db, &id)?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let dry = b.dry_run.unwrap_or(true);
    let fork = b.fork.unwrap_or(false);
    if s.kind.as_deref() == Some("history_only") {
        return Err(ApiError::bad("this session has no transcript to resume"));
    }
    let (agent, _) = st.agents.runnable(&s).map_err(ApiError::bad)?;
    let line = if fork {
        st.agents
            .fork_line(&s)
            .ok_or_else(|| ApiError::bad(format!("{} cannot fork a session", agent.name())))?
    } else {
        st.agents
            .resume_line(&s)
            .ok_or_else(|| ApiError::bad("this session has no resume command"))?
    };
    let plan = resume::plan_command(line, &resume::target(&st.cfg, &st.paths));
    let mut v = plan_json(&plan);
    if !dry {
        // restoring a transcript reads files and opening a tab or copying runs a program: neither on the async threads
        let note = blocking(move || {
            crate::archive::restore_if_missing(&db, &st.paths, &st.agents, &s)
                .map_err(|e| ApiError::bad(format!("{e:#}")))?;
            resume::execute(&plan).map_err(|e| ApiError::bad(format!("{e:#}")))
        })
        .await?;
        v["dry_run"] = json!(false);
        v["note"] = json!(note);
    }
    Ok(Json(v))
}

#[derive(Deserialize)]
pub(super) struct ConversationQ {
    limit: Option<usize>,
    before: Option<usize>,
}

pub(super) async fn api_conversation(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<ConversationQ>,
) -> ApiResult {
    let db = open_db(&st)?;
    if id.starts_with("idea:") {
        return Err(ApiError::bad("ideas have no conversation"));
    }
    let id = resolve(&db, &id)?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let limit = q
        .limit
        .unwrap_or(conversation::DEFAULT_LIMIT)
        .clamp(1, conversation::MAX_LIMIT);
    let agent = st.agents.of_session(&s).ok_or_else(|| {
        ApiError::not_found(format!(
            "the agent of this session ({}) is not enabled",
            s.agent
        ))
    })?;
    let all = conversation::read(&s, &st.paths, agent.as_ref())
        .map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(conversation::page(&all, limit, q.before).to_json()))
}

#[derive(Deserialize)]
pub(super) struct ExportQ {
    format: Option<String>,
}

/// One session as a download: a self-contained HTML page (or Markdown with `format=md`), secrets
/// masked, made from the archive copy when the agent deleted the transcript.
pub(super) async fn api_export(
    State(st): State<Shared>,
    Path(id): Path<String>,
    Query(q): Query<ExportQ>,
) -> Result<axum::response::Response, ApiError> {
    use crate::pack::{export_session, Format};
    use axum::http::header;
    use axum::response::IntoResponse;
    let (format, mime) = match q.format.as_deref().unwrap_or("html") {
        "html" => (Format::Html, "text/html; charset=utf-8"),
        "md" => (Format::Markdown, "text/markdown; charset=utf-8"),
        other => {
            return Err(ApiError::bad(format!(
                "unknown format '{other}' (html or md)"
            )))
        }
    };
    let db = open_db(&st)?;
    if id.starts_with("idea:") {
        return Err(ApiError::bad("ideas have no conversation to export"));
    }
    let id = resolve(&db, &id)?;
    let (name, text) = export_session(&db, &st.paths, &st.agents, &id, format)?;
    Ok((
        [
            (header::CONTENT_TYPE, mime.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox".to_string(),
            ),
        ],
        text,
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct SearchQ {
    q: Option<String>,
    limit: Option<usize>,
    all: Option<String>,
}

pub(super) async fn api_search(State(st): State<Shared>, Query(q): Query<SearchQ>) -> ApiResult {
    let db = open_db(&st)?;
    let words: Vec<String> =
        q.q.unwrap_or_default()
            .split_whitespace()
            .map(String::from)
            .collect();
    if !search::has_terms(&words) {
        return Ok(Json(json!({"results": []})));
    }
    let now = now_ms();
    let hidden = hidden_set(&db, &st.cfg, wants_all(&q.all))?;
    let hits = search::find_without(
        &db,
        &words,
        q.limit.unwrap_or(30).clamp(1, 100),
        true,
        &hidden,
        now,
    )?;
    let mut out = vec![];
    for h in &hits {
        out.push(json!({
            "card": load_card(&db, &h.session, now, st.card_ctx())?,
            "snippet": h.snippet,
            // `score` keeps 0.2's order (lower is better); `relevance` is the same number turned around
            "score": -h.score,
            "relevance": h.score,
        }));
    }
    Ok(Json(json!({"results": out})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ideas;
    use crate::server::test_support::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn column_move_and_auto() {
        let e = env();
        let now = now_ms();
        add_session(&e.db, "aaaaaaaa-1", "T", Some("done"), 4, now - 10_000);
        let (st, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/column",
                Some(json!({"column": "needs_you"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(c["column"], "needs_you");
        assert_eq!(c["auto_column"], "done");
        assert_eq!(c["manual"], true);
        let (_, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa/column",
                Some(json!({"column": "auto"})),
            ),
        )
        .await;
        assert_eq!(c["column"], "done");
        assert_eq!(c["manual"], false);
        let (st, _) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/column",
                Some(json!({"column": "nope"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (_, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/column",
                Some(json!({"column": "archive"})),
            ),
        )
        .await;
        assert_eq!(c["column"], "archive");
    }

    #[tokio::test]
    async fn sessions_cannot_be_moved_to_ideas() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "Older", Some("paused"), 4, now_ms());
        let (st, c) = call(
            &e,
            req(
                "POST",
                "/api/cards/aaaaaaaa-1/column",
                Some(json!({"column": "ideas"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert_eq!(c["error"], "sessions cannot be moved to Ideas");
    }

    #[tokio::test]
    async fn forget_endpoint_needs_a_confirm_and_refuses_an_open_session() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 5, now_ms());
        add_session(&e.db, "bbbbbbbb-2", "u", None, 5, now_ms());
        let uri = "/api/sessions/aaaaaaaa-1/forget";
        let here = |id: &str| e.db.session_by_id(id).unwrap().is_some();
        for body in [json!({}), json!({"confirm": false, "transcript": true})] {
            let (st, v) = call(&e, req("POST", uri, Some(body))).await;
            assert_eq!(st, StatusCode::BAD_REQUEST);
            assert!(v["error"].as_str().unwrap().contains("confirm"));
            assert!(here("aaaaaaaa-1"), "nothing goes without the confirm");
        }
        e.db.conn
            .execute(
                "UPDATE session SET live_status='idle' WHERE id='aaaaaaaa-1'",
                [],
            )
            .unwrap();
        let (st, v) = call(&e, req("POST", uri, Some(json!({"confirm": true})))).await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert_eq!(v["code"], "open_now");
        assert!(here("aaaaaaaa-1"));
        let (st, _) = call(
            &e,
            req(
                "POST",
                "/api/sessions/idea:1/forget",
                Some(json!({"confirm": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let mut r = req(
            "POST",
            "/api/sessions/bbbbbbbb-2/forget",
            Some(json!({"confirm": true})),
        );
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
        assert!(here("bbbbbbbb-2"));
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/sessions/bbbbbbbb-2/forget",
                Some(json!({"confirm": true, "transcript": true})),
            ),
        )
        .await;
        assert_eq!((st, v["ok"].as_bool()), (StatusCode::OK, Some(true)));
        assert!(!here("bbbbbbbb-2") && here("aaaaaaaa-1"));
    }

    #[tokio::test]
    async fn hidden_endpoint_hides_and_unhides() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 5, now_ms());
        let count = |v: &Value| -> usize {
            v["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["cards"].as_array().unwrap().len())
                .sum()
        };
        let uri = "/api/cards/aaaaaaaa-1/hidden";
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(count(&b), 1);
        let (st, v) = call(&e, req("POST", uri, Some(json!({"hidden": true})))).await;
        assert_eq!((st, v["id"].as_str()), (StatusCode::OK, Some("aaaaaaaa-1")));
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(count(&b), 0);
        let (_, b) = call(&e, req("GET", "/api/board?all=1", None)).await;
        assert_eq!(count(&b), 1);
        let (st, _) = call(&e, req("POST", uri, Some(json!({"hidden": false})))).await;
        assert_eq!(st, StatusCode::OK);
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(count(&b), 1);
        // errors: bad body, unknown session, idea, no token, foreign origin
        let (st, _) = call(&e, req("POST", uri, Some(json!({"hidden": "yes"})))).await;
        assert!(st.is_client_error());
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/cards/zzzzzzzz/hidden",
                Some(json!({"hidden": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(v["error"].is_string());
        let (st, _) = call(
            &e,
            req(
                "POST",
                "/api/cards/idea:1/hidden",
                Some(json!({"hidden": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let mut r = req("POST", uri, Some(json!({"hidden": true})));
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
        let mut r = req("POST", uri, Some(json!({"hidden": true})));
        r.headers_mut().remove("origin");
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn conversation_pages_back_from_the_newest() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        write_transcript(&e, "aaaaaaaa-1", 30);
        let (st, v) = call(&e, req("GET", "/api/cards/aaaaaaaa/conversation", None)).await;
        assert_eq!(st, StatusCode::OK);
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m.len(), 10);
        assert_eq!(
            (m[0]["role"].as_str(), m[0]["text"].as_str()),
            (Some("user"), Some("question 25"))
        );
        assert_eq!(m[9]["text"], "answer 29");
        assert_eq!(m[9]["role"], "assistant");
        assert_eq!(
            (&m[9]["tools"], &m[9]["truncated"]),
            (&json!(0), &json!(false))
        );
        assert!(m[9]["ts"].is_i64());
        assert_eq!((&v["more"], &v["next_before"]), (&json!(true), &json!(50)));
        let (_, v) = call(
            &e,
            req(
                "GET",
                "/api/cards/aaaaaaaa-1/conversation?limit=5&before=50",
                None,
            ),
        )
        .await;
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m.len(), 5);
        assert_eq!(m[0]["text"], "answer 22");
        assert_eq!(m[4]["text"], "answer 24");
        assert_eq!(v["next_before"], 45);
        // the first page of the history: nothing older
        let (_, v) = call(
            &e,
            req(
                "GET",
                "/api/cards/aaaaaaaa-1/conversation?limit=500&before=3",
                None,
            ),
        )
        .await;
        assert_eq!(v["messages"].as_array().unwrap().len(), 3);
        assert_eq!(
            (&v["more"], &v["next_before"]),
            (&json!(false), &Value::Null)
        );
        // limit is capped at 50
        let (_, v) = call(
            &e,
            req("GET", "/api/cards/aaaaaaaa-1/conversation?limit=500", None),
        )
        .await;
        assert_eq!(v["messages"].as_array().unwrap().len(), 50);
    }

    #[tokio::test]
    async fn conversation_errors() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        ideas::add(
            &e.db,
            &ideas::NewIdea {
                title: "i",
                notes: Some("n"),
                folder: Some("/tmp"),
                ..Default::default()
            },
        )
        .unwrap();
        let get = |u: &str| req("GET", u, None);
        assert_eq!(
            call(&e, get("/api/cards/zzzzzzzz/conversation")).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(&e, get("/api/cards/idea:1/conversation")).await.0,
            StatusCode::BAD_REQUEST
        );
        // known session, no transcript and no archive copy
        let (st, v) = call(&e, get("/api/cards/aaaaaaaa-1/conversation")).await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(v["error"].as_str().unwrap().contains("no transcript"));
        let mut r = get("/api/cards/aaaaaaaa-1/conversation");
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn export_downloads_one_page_even_when_the_agent_deleted_the_transcript() {
        use crate::paths::Paths;
        use crate::server::test_support::raw;
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "Fix the thing", None, 1, now_ms());
        let secret = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let t = e._tmp.path().join("aaaaaaaa-1.jsonl");
        std::fs::write(
            &t,
            format!("{{\"type\":\"user\",\"timestamp\":\"2026-11-08T10:00:00.000Z\",\"message\":{{\"content\":\"token {secret} please\"}}}}\n{{\"type\":\"assistant\",\"timestamp\":\"2026-11-08T10:00:05.000Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"all done\"}}]}}}}\n"),
        )
        .unwrap();
        e.db.conn
            .execute(
                "UPDATE session SET transcript_path=?1, project_dir='-tmp-demo' WHERE id='aaaaaaaa-1'",
                [t.to_string_lossy()],
            )
            .unwrap();
        // archived, then deleted by the agent: the page is made from the archive copy
        let paths = Paths {
            home: e._tmp.path().join("home"),
        };
        let cfg = crate::config::Config::with_claude_dir(&e._tmp.path().join("claude"));
        crate::archive::archive_session(&e.db, &paths, &crate::agents::enabled(&cfg), "aaaaaaaa-1")
            .unwrap();
        std::fs::remove_file(&t).unwrap();
        let (st, h, body) = raw(
            &e,
            req("GET", "/api/sessions/aaaaaaaa/export?format=html", None),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{body}");
        assert!(h["content-type"].to_str().unwrap().starts_with("text/html"));
        let cd = h["content-disposition"].to_str().unwrap();
        assert!(
            cd.starts_with("attachment; filename=\"eye-aaaaaaaa-fix-the-thing.html"),
            "{cd}"
        );
        assert!(
            body.contains("all done") && body.contains("[masked]"),
            "{body}"
        );
        assert!(!body.contains(secret));
        let (st, h, body) = raw(
            &e,
            req("GET", "/api/sessions/aaaaaaaa-1/export?format=md", None),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(h["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/markdown"));
        assert!(body.starts_with("# Fix the thing"));
        // errors: unknown session, idea, bad format, no token
        let get = |u: &str| req("GET", u, None);
        assert_eq!(
            call(&e, get("/api/sessions/zzzzzzzz/export")).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(&e, get("/api/sessions/idea:1/export")).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(&e, get("/api/sessions/aaaaaaaa-1/export?format=pdf"))
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
        let mut r = get("/api/sessions/aaaaaaaa-1/export");
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn resume_refuses_a_folder_with_control_characters_and_says_why() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        e.db.conn
            .execute(
                "UPDATE session SET cwd='/tmp/x\u{1b}[2Jrm' WHERE id='aaaaaaaa-1'",
                [],
            )
            .unwrap();
        let uri = "/api/cards/aaaaaaaa-1/resume";
        for body in [json!({"dry_run": true}), json!({"fork": true})] {
            let (st, r) = call(&e, req("POST", uri, Some(body))).await;
            assert_eq!(st, StatusCode::BAD_REQUEST);
            assert!(
                r["error"].as_str().unwrap().contains("control characters"),
                "{r}"
            );
        }
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert!(c["resume_cmd"].is_null() && c["resumable"] == false, "{c}");
    }

    #[tokio::test]
    async fn the_card_and_the_plan_say_what_resume_will_do() {
        let cfg = |setting: &str| {
            crate::config::Config::parse(&format!("[board]\nresume = \"{setting}\"")).unwrap()
        };
        let e = env_cfg(cfg("copy"));
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(c["resume_mode"], "copy");
        assert!(c["resume_terminal"].is_null(), "{c}");
        let uri = "/api/cards/aaaaaaaa-1/resume";
        let (_, r) = call(&e, req("POST", uri, None)).await;
        assert_eq!(r["mode"], "copy");
        assert!(r["terminal"].is_null() && r["applescript"].is_null(), "{r}");
        // the detail and the plan come from the same answer
        let e = env_cfg(cfg("auto"));
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        let (_, r) = call(&e, req("POST", uri, None)).await;
        assert_eq!(c["resume_mode"], r["mode"]);
        assert_eq!(c["resume_terminal"], r["terminal"]);
        // a command line of an idea that has no session yet
        let (_, i) = call(
            &e,
            req(
                "POST",
                "/api/ideas",
                Some(json!({"title": "x", "folder": "/tmp/x"})),
            ),
        )
        .await;
        let (_, c) = call(
            &e,
            req(
                "GET",
                &format!("/api/cards/{}", i["id"].as_str().unwrap()),
                None,
            ),
        )
        .await;
        assert!(
            ["iterm", "terminal", "copy"].contains(&c["resume_mode"].as_str().unwrap()),
            "{c}"
        );
    }

    #[tokio::test]
    async fn resume_can_fork() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        let uri = "/api/cards/aaaaaaaa-1/resume";
        let (_, r) = call(&e, req("POST", uri, Some(json!({"fork": true})))).await;
        assert_eq!(r["dry_run"], true);
        assert_eq!(
            r["command"],
            "cd '/tmp/demo' && claude --resume aaaaaaaa-1 --fork-session"
        );
        // `auto` comes to what this machine can do: an iTerm2 tab carries its AppleScript, the
        // other modes have none, and only a Linux terminal has a name
        let mode = r["mode"].as_str().unwrap();
        assert!(["iterm", "terminal", "copy"].contains(&mode), "{r}");
        assert_eq!(r["applescript"].is_string(), mode == "iterm", "{r}");
        if mode == "iterm" {
            assert!(r["applescript"]
                .as_str()
                .unwrap()
                .contains("--fork-session"));
        }
        assert!(r["terminal"].is_null() || mode == "terminal", "{r}");
        let (_, r) = call(
            &e,
            req("POST", uri, Some(json!({"fork": false, "dry_run": true}))),
        )
        .await;
        assert_eq!(r["command"], "cd '/tmp/demo' && claude --resume aaaaaaaa-1");
    }
}
