//! Board settings shared by every browser: theme and layout, kept in `ui.json`.

use super::api::{ApiError, ApiResult};
use super::{AppState, Shared};
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

/// Board settings the face mirrors here, so every browser and the installed app share them.
pub(super) const PREF_KEYS: &[&str] =
    &["eye.theme", "eye.ink.layout", "eye.ink.fold", "eye.accent"];

pub(super) fn prefs_path(st: &AppState) -> std::path::PathBuf {
    st.paths.home.join("ui.json")
}

pub(super) fn read_prefs(st: &AppState) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(prefs_path(st))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub(super) async fn api_prefs(State(st): State<Shared>) -> ApiResult {
    Ok(Json(Value::Object(read_prefs(&st))))
}

/// Merge `{key: "value"}` pairs for the known keys; anything else is refused.
pub(super) async fn api_prefs_set(State(st): State<Shared>, Json(b): Json<Value>) -> ApiResult {
    let obj = b
        .as_object()
        .ok_or_else(|| ApiError::bad("send a JSON object"))?;
    let mut prefs = read_prefs(&st);
    for (k, v) in obj {
        let v = v
            .as_str()
            .filter(|v| v.len() <= 2000)
            .ok_or_else(|| ApiError::bad(format!("{k}: give a short string")))?;
        if !PREF_KEYS.contains(&k.as_str()) {
            return Err(ApiError::bad(format!("unknown setting {k}")));
        }
        prefs.insert(k.clone(), json!(v));
    }
    crate::files::write_private(&prefs_path(&st), &Value::Object(prefs.clone()).to_string())
        .map_err(|e| ApiError::bad(format!("saving settings: {e:#}")))?;
    Ok(Json(Value::Object(prefs)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn prefs_round_trip_and_refuse_unknown_keys() {
        let e = env();
        let (st, _, body) = raw(&e, req("GET", "/api/prefs", None)).await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "{}"));
        let (st, _, _) = raw(
            &e,
            req(
                "POST",
                "/api/prefs",
                Some(json!({"eye.ink.layout": "grid"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, _, _) = raw(
            &e,
            req("POST", "/api/prefs", Some(json!({"eye.theme": "light"}))),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, _, body) = raw(&e, req("GET", "/api/prefs", None)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v, json!({"eye.ink.layout": "grid", "eye.theme": "light"}));
        for bad in [
            json!({"nope": "x"}),
            json!({"eye.theme": 3}),
            json!(["eye.theme"]),
        ] {
            let (st, _, _) = raw(&e, req("POST", "/api/prefs", Some(bad))).await;
            assert_eq!(st, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn the_accent_is_kept_and_an_old_file_without_it_still_reads() {
        let e = env();
        std::fs::write(
            e._tmp.path().join("home/ui.json"),
            r#"{"eye.theme": "dark", "eye.ink.layout": "lanes"}"#,
        )
        .unwrap();
        let (_, _, body) = raw(&e, req("GET", "/api/prefs", None)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v, json!({"eye.theme": "dark", "eye.ink.layout": "lanes"}));
        let (st, _, _) = raw(
            &e,
            req("POST", "/api/prefs", Some(json!({"eye.accent": "cobalt"}))),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, _, body) = raw(&e, req("GET", "/api/prefs", None)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v,
            json!({"eye.theme": "dark", "eye.ink.layout": "lanes", "eye.accent": "cobalt"})
        );
    }
}
