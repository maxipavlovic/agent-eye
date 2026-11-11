//! The reminder settings the board shows: whether a due reminder raises a desktop notice.

use super::api::{ApiError, ApiResult};
use super::Shared;
use crate::remind::notices;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct SettingsBody {
    notify: Option<bool>,
}

fn settings(st: &Shared) -> ApiResult {
    Ok(Json(json!({
        "notify": notices::is_on(&st.paths, st.cfg.remind.notify),
        "available": notices::available(),
    })))
}

pub(super) async fn api_remind_settings(State(st): State<Shared>) -> ApiResult {
    settings(&st)
}

/// Writes `[remind] notify` into `config.toml`; the daemon reads it on its next tick.
pub(super) async fn api_remind_settings_set(
    State(st): State<Shared>,
    Json(b): Json<SettingsBody>,
) -> ApiResult {
    let on = b
        .notify
        .ok_or_else(|| ApiError::bad("say `notify`: true or false"))?;
    notices::set(&st.paths, on).map_err(|e| ApiError::bad(format!("saving settings: {e:#}")))?;
    settings(&st)
}

#[cfg(test)]
mod tests {
    use crate::server::test_support::*;
    use axum::http::StatusCode;
    use serde_json::json;

    #[tokio::test]
    async fn notices_are_off_until_asked_and_the_answer_names_the_shape() {
        let e = env();
        let (st, v) = call(&e, req("GET", "/api/remind/settings", None)).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["notify"], false);
        assert!(v["available"].is_boolean());
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_post_switches_notices_on_and_off_and_keeps_the_config() {
        let e = env();
        let home = e._tmp.path().join("home");
        std::fs::write(
            home.join("config.toml"),
            "[board]\nport = 7500 # mine\n\n[remind]\nnotify = false # old\n",
        )
        .unwrap();
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/remind/settings",
                Some(json!({"notify": true})),
            ),
        )
        .await;
        assert_eq!((st, v["notify"].clone()), (StatusCode::OK, json!(true)));
        let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("port = 7500 # mine") && text.contains("notify = true    # old"));
        let (_, v) = call(&e, req("GET", "/api/remind/settings", None)).await;
        assert_eq!(v["notify"], true);
        let (_, v) = call(
            &e,
            req(
                "POST",
                "/api/remind/settings",
                Some(json!({"notify": false})),
            ),
        )
        .await;
        assert_eq!(v["notify"], false);
    }

    #[tokio::test]
    async fn a_post_with_a_config_that_does_not_parse_is_a_400_and_writes_nothing() {
        let e = env();
        let path = e._tmp.path().join("home").join("config.toml");
        std::fs::write(&path, "[remind\nnotify = false\n").unwrap();
        let (st, v) = call(
            &e,
            req(
                "POST",
                "/api/remind/settings",
                Some(json!({"notify": true})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("does not parse"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[remind\nnotify = false\n"
        );
    }

    #[tokio::test]
    async fn a_post_without_notify_is_refused() {
        let e = env();
        let (st, v) = call(&e, req("POST", "/api/remind/settings", Some(json!({})))).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("notify"));
    }

    #[tokio::test]
    async fn the_route_keeps_the_token_host_and_origin_rules() {
        let e = env();
        let mut r = req("GET", "/api/remind/settings", None);
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
        let mut r = req("GET", "/api/remind/settings", None);
        r.headers_mut()
            .insert("host", "evil.example:7420".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
        let mut r = req(
            "POST",
            "/api/remind/settings",
            Some(json!({"notify": true})),
        );
        r.headers_mut()
            .insert("origin", "http://evil.example".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
    }
}
