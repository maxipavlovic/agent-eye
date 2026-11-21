//! What a session left behind: list it, and act on one item after the page asked first. See
//! `docs/api.md`. Stop and remove run on stored items only, and never force anything.

use super::api::{blocking, open_db, resolve, ApiError, ApiResult};
use super::Shared;
use crate::loose::{self, LooseEnd};
use crate::resume;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize, Default)]
pub(super) struct ListQ {
    session: Option<String>,
}

fn items_json(items: &[LooseEnd]) -> Vec<Value> {
    items.iter().map(LooseEnd::to_json).collect()
}

/// The items of one session (or all), the count the person chose to ignore, and the one-line summary.
fn listing(db: &crate::db::Db, session: Option<&str>) -> Result<Value, ApiError> {
    let items = loose::list(db, session, false)?;
    let all = loose::list(db, session, true)?;
    Ok(json!({
        "items": items_json(&items),
        "ignored": all.len() - items.len(),
        "summary": loose::summary_line(&items),
    }))
}

pub(super) async fn api_loose(State(st): State<Shared>, Query(q): Query<ListQ>) -> ApiResult {
    let db = open_db(&st)?;
    let sid = match q.session.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => Some(resolve(&db, s)?),
        None => None,
    };
    Ok(Json(listing(&db, sid.as_deref())?))
}

#[derive(Deserialize)]
pub(super) struct KeyBody {
    key: String,
}

/// Run `act` on the stored item, then answer with what was done and what is left of the session.
async fn act_on(
    st: Shared,
    key: String,
    act: fn(&crate::db::Db, &dyn loose::Probe, &str) -> anyhow::Result<String>,
) -> ApiResult {
    let session = {
        let db = open_db(&st)?;
        loose::get(&db, &key)?
            .map(|i| i.session_id)
            .ok_or_else(|| ApiError::not_found(format!("no such loose end: {key}")))?
    };
    let note = blocking(move || {
        let db = open_db(&st)?;
        let probe = loose::System::with_roots(st.agents.scratch_roots());
        act(&db, &probe, &key).map_err(|e| ApiError::conflict("not_done", format!("{e:#}")))
    })
    .await?;
    Ok(Json(
        json!({"ok": true, "note": note, "session_id": session}),
    ))
}

pub(super) async fn api_loose_stop(State(st): State<Shared>, Json(b): Json<KeyBody>) -> ApiResult {
    act_on(st, b.key, loose::stop).await
}

pub(super) async fn api_loose_remove(
    State(st): State<Shared>,
    Json(b): Json<KeyBody>,
) -> ApiResult {
    act_on(st, b.key, loose::remove).await
}

pub(super) async fn api_loose_ignore(
    State(st): State<Shared>,
    Json(b): Json<KeyBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    loose::ignore(&db, &b.key, crate::util::now_ms())
        .map_err(|e| ApiError::not_found(format!("{e:#}")))?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct SendBody {
    session: String,
    dry_run: Option<bool>,
}

/// Resume the session with a first message that lists what it left, like the Resume button does:
/// a dry run (the default) only shows the command.
pub(super) async fn api_loose_send(
    State(st): State<Shared>,
    body: Option<Json<SendBody>>,
) -> ApiResult {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let db = open_db(&st)?;
    let id = resolve(&db, &b.session)?;
    let s = db
        .session_by_id(&id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let items = loose::list(&db, Some(&id), false)?;
    if items.is_empty() {
        return Err(ApiError::bad("nothing left behind to send back"));
    }
    if s.kind.as_deref() == Some("history_only") {
        return Err(ApiError::bad("this session has no transcript to resume"));
    }
    let line = st
        .agents
        .resume_line_with(&s, &loose::send_prompt(&items))
        .ok_or_else(|| ApiError::bad("this session has no resume command"))?;
    let plan = resume::plan_command(line, &resume::target(&st.cfg, &st.paths));
    let mut v = json!({"mode": plan.mode().as_str(), "command": plan.command,
                       "applescript": plan.applescript, "dry_run": b.dry_run.unwrap_or(true)});
    if !b.dry_run.unwrap_or(true) {
        let note = blocking(move || {
            crate::archive::restore_if_missing(&db, &st.paths, &st.agents, &s)
                .map_err(|e| ApiError::bad(format!("{e:#}")))?;
            resume::execute(&plan).map_err(|e| ApiError::bad(format!("{e:#}")))
        })
        .await?;
        v["note"] = json!(note);
    }
    Ok(Json(v))
}

#[cfg(test)]
mod tests {
    use crate::loose::{self, Kind, LooseEnd};
    use crate::server::test_support::*;
    use crate::util::now_ms;
    use axum::http::StatusCode;
    use serde_json::json;

    fn item(key: &str, sid: &str, kind: Kind, what: &str) -> LooseEnd {
        LooseEnd {
            key: key.into(),
            session_id: sid.into(),
            kind,
            line: format!("{what} is left"),
            detail: what.into(),
            since_ms: 1,
        }
    }

    #[tokio::test]
    async fn the_list_the_chip_count_and_ignore_agree() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "A", None, 3, now_ms());
        add_session(&e.db, "bbbbbbbb-1", "B", None, 3, now_ms());
        let a = item(
            "proc:1:1",
            "aaaaaaaa-1",
            Kind::Process,
            "node (pid 1, port 4321)",
        );
        let b = item(
            "unpushed:/r:abc",
            "aaaaaaaa-1",
            Kind::Unpushed,
            "3 commits in r",
        );
        loose::replace(&e.db, &[a.clone(), b.clone()], now_ms()).unwrap();
        // the card and the board carry the count
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(c["loose"], 2);
        assert_eq!(c["loose_ends"].as_array().unwrap().len(), 2);
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-1", None)).await;
        assert_eq!(c["loose"], 0);
        let (_, board) = call(&e, req("GET", "/api/board", None)).await;
        let counts: Vec<(String, i64)> = board["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["cards"].as_array().unwrap().clone())
            .map(|c| {
                (
                    c["id"].as_str().unwrap().to_string(),
                    c["loose"].as_i64().unwrap(),
                )
            })
            .collect();
        assert!(counts.contains(&("aaaaaaaa-1".into(), 2)));
        assert!(counts.contains(&("bbbbbbbb-1".into(), 0)));
        // the list, by prefix
        let (st, v) = call(&e, req("GET", "/api/loose?session=aaaaaaaa", None)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
        assert_eq!(v["items"][0]["action"], "stop");
        assert_eq!(v["items"][0]["confirm"], "Stop node (pid 1, port 4321)?");
        assert_eq!(v["items"][1]["action"], "push");
        assert!(v["items"][1]["command"]
            .as_str()
            .unwrap()
            .ends_with("git push --all"));
        assert_eq!(
            v["summary"],
            "Left behind: 1 server (port 4321), 3 commits not pushed"
        );
        // ignore drops the count; all ignored: no chip
        let (st, _) = call(
            &e,
            req(
                "POST",
                "/api/loose/ignore",
                Some(json!({"key": "proc:1:1"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(c["loose"], 1);
        call(
            &e,
            req("POST", "/api/loose/ignore", Some(json!({"key": b.key}))),
        )
        .await;
        let (_, v) = call(&e, req("GET", "/api/loose?session=aaaaaaaa", None)).await;
        assert_eq!(
            (v["items"].as_array().unwrap().len(), v["ignored"].as_i64()),
            (0, Some(2))
        );
        let (st, _) = call(
            &e,
            req("POST", "/api/loose/ignore", Some(json!({"key": "nope"}))),
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn acting_routes_need_the_guard_and_a_stored_item() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "A", None, 3, now_ms());
        for uri in [
            "/api/loose/stop",
            "/api/loose/remove",
            "/api/loose/ignore",
            "/api/loose/send",
        ] {
            let mut r = req(
                "POST",
                uri,
                Some(json!({"key": "x", "session": "aaaaaaaa"})),
            );
            r.headers_mut().remove("origin");
            assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN, "{uri}");
            let mut r = req("POST", uri, Some(json!({"key": "x"})));
            r.headers_mut().remove("x-eye-token");
            assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED, "{uri}");
        }
        // a key no scan found is refused, and nothing is run
        for uri in ["/api/loose/stop", "/api/loose/remove"] {
            let (st, _) = call(&e, req("POST", uri, Some(json!({"key": "proc:1:1"})))).await;
            assert_eq!(st, StatusCode::NOT_FOUND, "{uri}");
        }
        // nothing to send back
        let (st, _) = call(
            &e,
            req(
                "POST",
                "/api/loose/send",
                Some(json!({"session": "aaaaaaaa"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stopping_a_process_that_changed_owner_is_refused_with_a_reason() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "A", None, 3, now_ms());
        // our own pid with a start time that is not ours
        let key = format!("proc:{}:1", std::process::id());
        loose::replace(
            &e.db,
            &[item(&key, "aaaaaaaa-1", Kind::Process, "x (pid 1)")],
            now_ms(),
        )
        .unwrap();
        let (st, v) = call(
            &e,
            req("POST", "/api/loose/stop", Some(json!({"key": key}))),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT);
        assert!(
            v["error"].as_str().unwrap().contains("another process"),
            "{v}"
        );
        assert_eq!(v["code"], "not_done");
    }

    #[tokio::test]
    async fn send_back_shows_the_command_and_stays_dry_by_default() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "A", None, 3, now_ms());
        loose::replace(
            &e.db,
            &[item(
                "uncommitted:/r",
                "aaaaaaaa-1",
                Kind::Uncommitted,
                "2 modified files in r",
            )],
            now_ms(),
        )
        .unwrap();
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/loose/send",
                Some(json!({"session": "aaaaaaaa"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["dry_run"], true);
        let cmd = v["command"].as_str().unwrap();
        assert!(
            cmd.contains("claude --resume aaaaaaaa-1 'Before you call this finished"),
            "{cmd}"
        );
        assert!(v.get("note").is_none());
    }
}
