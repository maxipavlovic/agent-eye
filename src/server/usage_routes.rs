//! `GET /api/usage`: usage over the last days, for the board's USAGE panel.

use super::api::{blocking, open_db, ApiError, ApiResult};
use super::Shared;
use crate::usage::report::{self, By};
use crate::util::now_ms;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct UsageQ {
    days: Option<i64>,
    by: Option<String>,
}

/// The report (see `docs/api.md`): rows grouped by `by` (default `project`) over `days` (default
/// 7, at most 366), the other groupings, the four-week trend and the plan limits.
pub(super) async fn api_usage(State(st): State<Shared>, Query(q): Query<UsageQ>) -> ApiResult {
    let days = q.days.unwrap_or(7);
    if !(1..=366).contains(&days) {
        return Err(ApiError::bad("days takes a number from 1 to 366"));
    }
    let by = match q.by.as_deref() {
        None => By::Project,
        Some(b) => By::parse(b)
            .ok_or_else(|| ApiError::bad("by takes project, agent, model or session"))?,
    };
    blocking(move || {
        let db = open_db(&st)?;
        let now = now_ms();
        let r = report::build(
            &db.conn,
            &st.cfg.usage,
            &st.agents,
            days,
            by,
            now,
            &crate::worked::summary::local_offset_ms,
        )?;
        Ok(Json(r.json(now)))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use crate::usage::store;
    use crate::usage::UsageMsg;
    use axum::http::StatusCode;

    fn msg(model: &str, input: i64) -> UsageMsg {
        UsageMsg {
            ts: crate::util::now_ms(),
            model: model.into(),
            input,
            output: 0,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_empty_index_answers_with_an_empty_report() {
        let e = env();
        let (s, v) = call(&e, req("GET", "/api/usage?days=7&by=project", None)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["rows"], serde_json::json!([]));
        assert_eq!(v["totals"]["total"], 0);
        assert_eq!(v["weeks"].as_array().unwrap().len(), 4);
        assert!(v["estimate"].as_str().unwrap().contains("not your bill"));
    }

    #[tokio::test]
    async fn rows_follow_the_grouping_and_an_unknown_model_has_no_cost() {
        let e = env();
        add_session(&e.db, "a1", "one", None, 1, 1);
        store::replace(
            &e.db.conn,
            "a1",
            &[
                msg("claude-opus-5-5", 1_000_000),
                msg("claude-from-the-future", 10),
            ],
        )
        .unwrap();
        let (_, v) = call(&e, req("GET", "/api/usage?days=7&by=model", None)).await;
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "claude-opus-5-5");
        assert_eq!(rows[0]["cost_usd"], 4.0);
        assert_eq!(rows[1]["cost_usd"], serde_json::Value::Null);
        assert_eq!(rows[1]["unpriced_tokens"], 10);
        let (_, v) = call(&e, req("GET", "/api/usage?by=session", None)).await;
        assert_eq!(v["rows"][0]["id"], "a1");
        assert_eq!(v["top_sessions"][0]["id"], "a1");
    }

    #[tokio::test]
    async fn a_bad_window_or_grouping_is_a_400() {
        let e = env();
        for q in ["days=0", "days=999", "by=colour"] {
            let (s, v) = call(&e, req("GET", &format!("/api/usage?{q}"), None)).await;
            assert_eq!(s, StatusCode::BAD_REQUEST, "{q}");
            assert!(v["error"].is_string());
        }
    }
}
