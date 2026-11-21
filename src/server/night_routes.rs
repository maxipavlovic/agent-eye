//! The night shift on the board: state and settings, planning, cancelling, changing a limit,
//! discarding a finished run and opening its folder. See `docs/night.md` and `docs/api.md`. The
//! report of a run is untrusted output: it is sent as text and the face renders it as text.

use super::api::{blocking, open_db, resolve, ApiError, ApiResult};
use super::Shared;
use crate::config::Config;
use crate::night::{self, clock, Limit, PlanOpts};
use crate::util::now_ms;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// The settings now: the file is read again, so a switch made elsewhere counts at once.
fn fresh_cfg(st: &super::AppState) -> Config {
    std::fs::read_to_string(st.paths.config_path())
        .ok()
        .and_then(|t| Config::parse(&t).ok())
        .unwrap_or_else(|| st.cfg.clone())
}

fn state_json(st: &super::AppState, db: &crate::db::Db) -> Result<Value, ApiError> {
    let cfg = fresh_cfg(st);
    let now = now_ms();
    let w = clock::Window::parse(&cfg.night.window).unwrap_or_default();
    let runs = night::list(db, false)?;
    let open: Vec<Value> = runs
        .iter()
        .filter(|r| r.is_open())
        .map(|r| r.to_json(now))
        .collect();
    let recent: Vec<Value> = runs
        .iter()
        .filter(|r| !r.is_open())
        .take(20)
        .map(|r| r.to_json(now))
        .collect();
    Ok(json!({
        "enabled": cfg.night.enabled,
        "window": w.text(),
        "parallel": cfg.night.parallel,
        "quiet_minutes": cfg.night.quiet_minutes,
        "in_window": clock::in_window(&w, now, &clock::local_offset),
        "planned": night::planned_count(db)?,
        "open": open,
        "recent": recent,
    }))
}

pub(super) async fn api_night(State(st): State<Shared>) -> ApiResult {
    let db = open_db(&st)?;
    Ok(Json(state_json(&st, &db)?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct SettingsBody {
    enabled: Option<bool>,
    window: Option<String>,
    parallel: Option<u32>,
}

pub(super) async fn api_night_settings(
    State(st): State<Shared>,
    Json(b): Json<SettingsBody>,
) -> ApiResult {
    let path = st.paths.config_path();
    let bad = |e: anyhow::Error| ApiError::bad(format!("{e:#}"));
    if let Some(w) = &b.window {
        let w = clock::Window::parse(w).map_err(ApiError::bad)?;
        crate::config::write_night(&path, "window", &format!("{:?}", w.text())).map_err(bad)?;
    }
    if let Some(n) = b.parallel {
        if !(1..=8).contains(&n) {
            return Err(ApiError::bad("parallel is a number from 1 to 8"));
        }
        crate::config::write_night(&path, "parallel", &n.to_string()).map_err(bad)?;
    }
    if let Some(on) = b.enabled {
        crate::config::write_night(&path, "enabled", &on.to_string()).map_err(bad)?;
    }
    let db = open_db(&st)?;
    Ok(Json(state_json(&st, &db)?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct PlanBody {
    idea_id: Option<i64>,
    session_id: Option<String>,
    prompt: Option<String>,
    limit: Option<String>,
    agent: Option<String>,
    switch: bool,
}

pub(super) async fn api_night_plan(State(st): State<Shared>, Json(b): Json<PlanBody>) -> ApiResult {
    let limit = match b.limit.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => Some(Limit::parse(l).map_err(|e| ApiError::bad(format!("{e:#}")))?),
        None => None,
    };
    let opts = PlanOpts {
        agent: b.agent.clone(),
        limit,
        switch: b.switch,
        prompt: b.prompt.clone(),
    };
    let run = {
        let st = st.clone();
        blocking(move || {
            let db = open_db(&st)?;
            let cfg = fresh_cfg(&st);
            let r = match (b.idea_id, b.session_id.as_deref()) {
                (Some(i), None) => night::plan_idea(&db, &cfg, &st.agents, i, &opts),
                (None, Some(s)) => night::plan_session(&db, &cfg, &st.agents, s, &opts),
                _ => Err(anyhow::anyhow!(
                    "plan an idea (idea_id) or a session (session_id)"
                )),
            };
            r.map_err(|e| ApiError::bad(format!("{e:#}")))
        })
        .await?
    };
    Ok(Json(run.to_json(now_ms())))
}

pub(super) async fn api_night_cancel(State(st): State<Shared>, Path(id): Path<i64>) -> ApiResult {
    let db = open_db(&st)?;
    night::cancel(&db, id).map_err(|e| ApiError::conflict("not_done", format!("{e:#}")))?;
    Ok(Json(state_json(&st, &db)?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct LimitBody {
    limit: Option<String>,
}

pub(super) async fn api_night_limit(
    State(st): State<Shared>,
    Path(id): Path<i64>,
    Json(b): Json<LimitBody>,
) -> ApiResult {
    let db = open_db(&st)?;
    let limit = match b.limit.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => Some(Limit::parse(l).map_err(|e| ApiError::bad(format!("{e:#}")))?),
        None => None,
    };
    night::set_limit(&db, &st.agents, id, limit).map_err(|e| ApiError::bad(format!("{e:#}")))?;
    Ok(Json(state_json(&st, &db)?))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct DiscardBody {
    /// The page asked and the person said yes.
    confirm: bool,
    /// Uncommitted changes in the worktree may be lost.
    lose_changes: bool,
}

pub(super) async fn api_night_discard(
    State(st): State<Shared>,
    Path(id): Path<i64>,
    Json(b): Json<DiscardBody>,
) -> ApiResult {
    if !b.confirm {
        return Err(ApiError::bad(
            "discarding needs the person's confirmation (confirm: true)",
        ));
    }
    let said = blocking(move || {
        let db = open_db(&st)?;
        let run = night::get(&db, id)?.ok_or_else(|| ApiError::not_found("no such run"))?;
        let said = night::iso::discard(
            &crate::loose::System::new(),
            &st.paths.home.join("night"),
            &run,
            b.lose_changes,
        )
        .map_err(|e| ApiError::conflict("not_done", format!("{e:#}")))?;
        db.conn.execute(
            "UPDATE night_run SET worktree=NULL, branch=NULL WHERE id=?1",
            [id],
        )?;
        Ok(said)
    })
    .await?;
    Ok(Json(json!({"ok": true, "note": said})))
}

pub(super) async fn api_night_open(State(st): State<Shared>, Path(id): Path<i64>) -> ApiResult {
    let db = open_db(&st)?;
    let run = night::get(&db, id)?.ok_or_else(|| ApiError::not_found("no such run"))?;
    let dir = run
        .worktree
        .filter(|d| std::path::Path::new(d).is_dir())
        .ok_or_else(|| ApiError::bad("this run has no folder any more"))?;
    blocking(move || crate::platform::open_url(&dir).map_err(|e| ApiError::bad(format!("{e:#}"))))
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// The run behind a session card (the session the run created), for the NIGHT RUN section.
pub(super) fn run_of_session(db: &crate::db::Db, session_id: &str) -> anyhow::Result<Value> {
    let Some(r) = night::of_session(db, session_id)? else {
        return Ok(Value::Null);
    };
    let mut v = r.to_json(now_ms());
    v["tokens"] = json!(night::sched::run_tokens(db, session_id));
    v["uncommitted"] = json!(
        r.worktree.is_some() && night::iso::has_uncommitted(&crate::loose::System::new(), &r)
    );
    Ok(v)
}

/// `GET /api/night/session/:id`.
pub(super) async fn api_night_of_session(
    State(st): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult {
    let db = open_db(&st)?;
    let id = resolve(&db, &id)?;
    let v = blocking(move || {
        let db = open_db(&st)?;
        Ok(run_of_session(&db, &id)?)
    })
    .await?;
    Ok(Json(json!({"run": v})))
}
