//! `GET /api/clock`: the steering clock's report, for the board's USAGE panel.

use super::api::{blocking, open_db, ApiError, ApiResult};
use super::Shared;
use crate::clock::report::{self, MAX_WEEKS};
use crate::util::now_ms;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ClockQ {
    weeks: Option<i64>,
}

/// The report (see `docs/api.md`) for the last `weeks` weeks (default 4, 1 to 12), made with the
/// settings in the file now, so a change in the gear counts at once.
pub(super) async fn api_clock(State(st): State<Shared>, Query(q): Query<ClockQ>) -> ApiResult {
    let weeks = q.weeks.unwrap_or(4);
    if !(1..=MAX_WEEKS).contains(&weeks) {
        return Err(ApiError::bad(format!(
            "weeks takes a number from 1 to {MAX_WEEKS}"
        )));
    }
    blocking(move || {
        let db = open_db(&st)?;
        let cfg = st.cfg.fresh(&st.paths.config_path());
        let r = report::build(
            &db.conn,
            &cfg,
            &st.agents,
            now_ms(),
            weeks,
            &crate::worked::summary::local_offset_ms,
        )?;
        Ok(Json(r.json()))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn an_empty_index_answers_with_a_whole_empty_report() {
        let e = env();
        let (s, v) = call(&e, req("GET", "/api/clock", None)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["hours"].as_array().unwrap().len(), 24);
        assert_eq!(v["since_break"].as_array().unwrap().len(), 4);
        assert_eq!(v["verdict"]["kind"], "too_few");
        assert_eq!(v["bad"], serde_json::json!([]));
        assert!(v["now"]["hint"].is_null());
    }

    #[tokio::test]
    async fn turns_of_the_window_are_counted() {
        let e = env();
        add_session(&e.db, "a1", "one", None, 1, 1);
        let now = crate::util::now_ms();
        for (ts, kind) in [
            (now - 1000, "prompt"),
            (now - 900, "tool"),
            (now - 800, "tool_error"),
        ] {
            e.db.conn
                .execute(
                    "INSERT INTO turn(session_id, ts, kind, n) VALUES('a1',?1,?2,0)",
                    rusqlite::params![ts, kind],
                )
                .unwrap();
        }
        let (_, v) = call(&e, req("GET", "/api/clock?weeks=1", None)).await;
        assert_eq!(v["prompts"], 1);
        let hours = v["hours"].as_array().unwrap();
        let sum = |k: &str| hours.iter().map(|h| h[k].as_i64().unwrap()).sum::<i64>();
        assert_eq!((sum("tool_calls"), sum("tool_errors")), (1, 1));
    }

    #[tokio::test]
    async fn a_bad_window_is_a_400() {
        let e = env();
        for q in ["weeks=0", "weeks=13"] {
            let (s, v) = call(&e, req("GET", &format!("/api/clock?{q}"), None)).await;
            assert_eq!(s, StatusCode::BAD_REQUEST, "{q}");
            assert!(v["error"].is_string(), "{q}: {v}");
        }
    }
}
