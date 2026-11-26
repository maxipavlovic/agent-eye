//! `GET /api/usage`: usage over the last days, for the board's USAGE panel.

use super::api::{blocking, open_db, ApiError, ApiResult};
use super::Shared;
use crate::usage::report::{self, By};
use crate::usage::series;
use crate::util::now_ms;
use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct UsageQ {
    days: Option<i64>,
    by: Option<String>,
    series: Option<String>,
}

/// The report (see `docs/api.md`): rows grouped by `by` (default `project`) over `days` (default
/// 7, at most 366), the other groupings, the four-week trend and the plan limits; with `series`
/// (`project` or `model`) also the cost of each day split that way.
pub(super) async fn api_usage(State(st): State<Shared>, Query(q): Query<UsageQ>) -> ApiResult {
    let days = q.days.unwrap_or(7);
    if !(1..=366).contains(&days) {
        return Err(ApiError::bad("days takes a number from 1 to 366"));
    }
    let by = match q.by.as_deref() {
        None => By::Project,
        Some(b) => By::parse(b)
            .ok_or_else(|| ApiError::bad("by takes project, folder, agent, model or session"))?,
    };
    let series_by = match q.series.as_deref() {
        None => None,
        Some(b) => match By::parse(b) {
            Some(by @ (By::Project | By::Model)) => Some(by),
            _ => return Err(ApiError::bad("series takes project or model")),
        },
    };
    blocking(move || {
        let db = open_db(&st)?;
        let now = now_ms();
        let offset = &crate::worked::summary::local_offset_ms;
        let r = report::build(&db.conn, &st.cfg.usage, &st.agents, days, by, now, offset)?;
        let mut v = r.json(now);
        if let Some(by) = series_by {
            let s = series::build(&db.conn, &st.cfg.usage, days, by, now, offset)?;
            v["series"] = s.json();
        }
        Ok(Json(v))
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
    async fn the_series_field_comes_only_when_asked_for() {
        let e = env();
        add_session(&e.db, "a1", "one", None, 1, 1);
        store::replace(&e.db.conn, "a1", &[msg("claude-opus-5-5", 1_000_000)]).unwrap();
        let (_, v) = call(&e, req("GET", "/api/usage?days=14", None)).await;
        assert!(v.get("series").is_none());
        let (s, v) = call(&e, req("GET", "/api/usage?days=14&series=model", None)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["series"]["by"], "model");
        assert_eq!(v["series"]["keys"], serde_json::json!(["claude-opus-5-5"]));
        assert_eq!(v["series"]["days"].as_array().unwrap().len(), 14);
        let (_, v) = call(&e, req("GET", "/api/usage?days=30&series=project", None)).await;
        assert_eq!(v["series"]["days"].as_array().unwrap().len(), 30);
    }

    #[tokio::test]
    async fn a_bad_window_or_grouping_is_a_400() {
        let e = env();
        for q in ["days=0", "days=999", "by=colour", "series=folder"] {
            let (s, v) = call(&e, req("GET", &format!("/api/usage?{q}"), None)).await;
            assert_eq!(s, StatusCode::BAD_REQUEST, "{q}");
            assert!(v["error"].is_string());
        }
    }
}
