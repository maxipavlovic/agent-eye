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
        "isolation": cfg.night.isolation,
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
    isolation: Option<String>,
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
        isolation: b.isolation.clone(),
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

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct BringBody {
    /// The page asked and the person said yes.
    confirm: bool,
}

/// `POST /api/night/:id/bring`: fetch a repo copy's night branch into the original repo.
pub(super) async fn api_night_bring(
    State(st): State<Shared>,
    Path(id): Path<i64>,
    Json(b): Json<BringBody>,
) -> ApiResult {
    if !b.confirm {
        return Err(ApiError::bad(
            "bringing a branch into your repo needs the person's confirmation (confirm: true)",
        ));
    }
    let said = blocking(move || {
        let db = open_db(&st)?;
        let run = night::get(&db, id)?.ok_or_else(|| ApiError::not_found("no such run"))?;
        night::iso::bring(&crate::loose::System::new(), &run)
            .map_err(|e| ApiError::conflict("not_done", format!("{e:#}")))
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

#[cfg(test)]
mod tests {
    use crate::server::test_support::*;
    use crate::util::now_ms;
    use axum::http::StatusCode;
    use serde_json::json;

    fn idea_with(e: &Env, prompt: Option<&str>) -> i64 {
        let dir = e._tmp.path().join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        crate::ideas::add(
            &e.db,
            &crate::ideas::NewIdea {
                title: "t",
                prompt,
                folder: dir.to_str(),
                ..Default::default()
            },
        )
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn the_state_starts_off_and_empty_and_settings_land_in_the_config() {
        let e = env();
        let (st, v) = call(&e, req("GET", "/api/night", None)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(
            (
                v["enabled"].clone(),
                v["window"].clone(),
                v["planned"].clone()
            ),
            (json!(false), json!("01:00-07:00"), json!(0))
        );
        assert_eq!(v["open"], json!([]));
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/night/settings",
                Some(json!({"enabled": true, "window": "23:00-05:00", "parallel": 3})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(
            (
                v["enabled"].clone(),
                v["window"].clone(),
                v["parallel"].clone()
            ),
            (json!(true), json!("23:00-05:00"), json!(3))
        );
        let cfg = std::fs::read_to_string(e._tmp.path().join("home/config.toml")).unwrap();
        assert!(
            cfg.contains("window = \"23:00-05:00\"") && cfg.contains("parallel = 3"),
            "{cfg}"
        );
        // bad values are refused and change nothing
        for body in [
            json!({"window": "7-9"}),
            json!({"parallel": 0}),
            json!({"parallel": 99}),
        ] {
            let (st, v) = call(&e, req("POST", "/api/night/settings", Some(body))).await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        }
        let (_, v) = call(&e, req("GET", "/api/night", None)).await;
        assert_eq!(v["window"], "23:00-05:00");
    }

    #[tokio::test]
    async fn planning_cancelling_and_changing_a_limit() {
        let e = env();
        let id = idea_with(&e, Some("write a readme"));
        let (st, run) = call(
            &e,
            req(
                "POST",
                "/api/night/plan",
                Some(json!({"idea_id": id, "limit": "2M"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{run}");
        assert_eq!(
            (run["status"].clone(), run["limit"]["text"].clone()),
            (json!("planned"), json!("2M tokens"))
        );
        let rid = run["id"].as_i64().unwrap();
        let (_, v) = call(&e, req("GET", "/api/night", None)).await;
        assert_eq!(v["planned"], 1);
        let (st, v) = call(
            &e,
            req(
                "POST",
                &format!("/api/night/{rid}/limit"),
                Some(json!({"limit": "90m"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["open"][0]["limit"]["text"], "90 minutes");
        // a weekly limit eye cannot check is refused with the reason
        let (st, v) = call(
            &e,
            req(
                "POST",
                &format!("/api/night/{rid}/limit"),
                Some(json!({"limit": "80%"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(
            v["error"].as_str().unwrap().contains("no weekly plan data"),
            "{v}"
        );
        let (st, v) = call(&e, req("POST", &format!("/api/night/{rid}/cancel"), None)).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        assert_eq!(v["planned"], 0);
        // a second cancel is a conflict
        let (st, _) = call(&e, req("POST", &format!("/api/night/{rid}/cancel"), None)).await;
        assert_eq!(st, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn a_plan_that_cannot_run_says_why() {
        let e = env();
        let id = idea_with(&e, None);
        let (st, v) = call(
            &e,
            req("POST", "/api/night/plan", Some(json!({"idea_id": id}))),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("no prompt"), "{v}");
        let (st, v) = call(
            &e,
            req("POST", "/api/night/plan", Some(json!({"session_id": "x"}))),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{v}");
        let (st, _) = call(&e, req("POST", "/api/night/plan", Some(json!({})))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_run_session_carries_the_night_chip_and_the_run_and_discard_needs_a_yes() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "Night one", None, 3, now_ms());
        e.db.conn
            .execute(
                "INSERT INTO night_run(kind, agent, folder, prompt, night, status, run_session_id, report, created_at)
                 VALUES('idea','claude','/x','p','2026-11-21','done','aaaaaaaa-1','The report <b>x</b>',1)",
                [],
            )
            .unwrap();
        let (_, c) = call(&e, req("GET", "/api/cards/aaaaaaaa-1", None)).await;
        assert_eq!(c["night_run"]["status"], "done");
        assert_eq!(c["night_run"]["report"], "The report <b>x</b>");
        assert_eq!(c["night_run"]["tokens"], 0);
        let (_, board) = call(&e, req("GET", "/api/board", None)).await;
        let chip: Vec<_> = board["columns"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|c| c["cards"].as_array().unwrap().clone())
            .filter(|c| c["id"] == "aaaaaaaa-1")
            .map(|c| c["night"].clone())
            .collect();
        assert_eq!(chip, vec![json!({"id": 1, "status": "done"})]);
        let (_, v) = call(&e, req("GET", "/api/night/session/aaaaaaaa", None)).await;
        assert_eq!(v["run"]["id"], 1);
        // another session has none
        add_session(&e.db, "bbbbbbbb-1", "Plain", None, 3, now_ms());
        let (_, c) = call(&e, req("GET", "/api/cards/bbbbbbbb-1", None)).await;
        assert!(c["night_run"].is_null() && c["night"].is_null());
        // discard needs the page's yes, and a run with no folder has nothing to discard
        let (st, _) = call(&e, req("POST", "/api/night/1/discard", Some(json!({})))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/night/1/discard",
                Some(json!({"confirm": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::CONFLICT, "{v}");
    }
    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let o = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    #[tokio::test]
    async fn the_plan_takes_an_isolation_and_bring_needs_a_yes_and_lands_the_branch() {
        let e = env();
        let orig = e._tmp.path().join("proj");
        std::fs::create_dir_all(&orig).unwrap();
        git(&orig, &["init", "-q", "-b", "main"]);
        std::fs::write(orig.join("a.txt"), "one\n").unwrap();
        git(&orig, &["add", "."]);
        git(&orig, &["commit", "-qm", "first"]);
        let id = crate::ideas::add(
            &e.db,
            &crate::ideas::NewIdea {
                title: "t",
                prompt: Some("p"),
                folder: orig.to_str(),
                ..Default::default()
            },
        )
        .unwrap()
        .id;
        let (_, st) = call(&e, req("GET", "/api/night", None)).await;
        assert_eq!(st["isolation"], "worktree");
        let (s, v) = call(
            &e,
            req(
                "POST",
                "/api/night/plan",
                Some(json!({"idea_id": id, "isolation": "copy"})),
            ),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["isolation"], "copy");
        let (_, v) = call(
            &e,
            req("POST", "/api/night/plan", Some(json!({"idea_id": id}))),
        )
        .await;
        assert_eq!(v["isolation"], "worktree");
        // a finished repo copy
        let i = crate::night::iso::prepare(
            &crate::loose::System::new(),
            &e._tmp.path().join("eye/night/1"),
            &orig,
            "2026-11-21",
            "x",
            1,
            crate::night::Mode::Copy,
        )
        .unwrap();
        git(&i.root, &["commit", "-q", "--allow-empty", "-m", "night"]);
        let b = i.branch.clone().unwrap();
        e.db.conn
            .execute(
                "UPDATE night_run SET status='done', isolation='copy', worktree=?2, branch=?3, base_ref=?4 WHERE id=?1",
                rusqlite::params![1, i.root.to_string_lossy(), b, i.base_ref],
            )
            .unwrap();
        let (s, _) = call(&e, req("POST", "/api/night/1/bring", Some(json!({})))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(git(&orig, &["branch", "--list", &b]).is_empty());
        let (s, v) = call(
            &e,
            req("POST", "/api/night/1/bring", Some(json!({"confirm": true}))),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(!git(&orig, &["branch", "--list", &b]).is_empty());
        // the branch is in the original now: a second bring is refused
        let (s, _) = call(
            &e,
            req("POST", "/api/night/1/bring", Some(json!({"confirm": true}))),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT);
    }
}
