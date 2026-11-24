//! Board settings shared by every browser: theme, accent, look and each look's layout, kept in `ui.json`.

use super::api::{ApiError, ApiResult};
use super::{AppState, Shared};
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

/// Board settings every look shares; the face mirrors them here, so every browser and the installed app share them.
const SHARED_PREFS: &[&str] = &["eye.theme", "eye.accent", "eye.look"];

/// What a look keeps for itself, as `eye.<look>.<setting>` (`eye.ink.layout`, `eye.iris.fold`).
const LOOK_PREFS: &[&str] = &["layout", "fold", "panel"];

/// At most this many settings in `ui.json`, so no client can grow it without end (two looks use 8).
const MAX_PREFS: usize = 64;

/// A shared setting, or a look's own with a short lowercase look name (a letter, then letters or digits, 16 at most).
pub(super) fn is_pref_key(k: &str) -> bool {
    if SHARED_PREFS.contains(&k) {
        return true;
    }
    let Some((look, setting)) = k.strip_prefix("eye.").and_then(|r| r.split_once('.')) else {
        return false;
    };
    LOOK_PREFS.contains(&setting)
        && (1..=16).contains(&look.len())
        && look.starts_with(|c: char| c.is_ascii_lowercase())
        && look
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

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
        if !is_pref_key(k) {
            return Err(ApiError::bad(format!("unknown setting {k}")));
        }
        if !prefs.contains_key(k) && prefs.len() >= MAX_PREFS {
            return Err(ApiError::bad(format!(
                "too many board settings ({MAX_PREFS}); remove some from ui.json"
            )));
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

    #[test]
    fn a_look_keeps_its_own_settings_and_odd_names_are_refused() {
        for ok in [
            "eye.theme",
            "eye.accent",
            "eye.look",
            "eye.ink.layout",
            "eye.ink.fold",
            "eye.ink.panel",
            "eye.iris.layout",
            "eye.iris2.fold",
        ] {
            assert!(is_pref_key(ok), "{ok}");
        }
        for bad in [
            "eye.iris.colour",
            "eye.Iris.layout",
            "eye.2iris.layout",
            "eye.ir-is.layout",
            "eye..layout",
            "eye.iris",
            "eye.a.b.layout",
            "eye.abcdefghijklmnopq.layout",
            "iris.layout",
            "eye.looks",
        ] {
            assert!(!is_pref_key(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn the_chosen_look_and_its_layout_round_trip() {
        let e = env();
        let (st, _, _) = raw(
            &e,
            req(
                "POST",
                "/api/prefs",
                Some(json!({"eye.look": "iris", "eye.iris.layout": "grid"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (_, _, body) = raw(&e, req("GET", "/api/prefs", None)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v, json!({"eye.look": "iris", "eye.iris.layout": "grid"}));
        let (st, _, _) = raw(
            &e,
            req("POST", "/api/prefs", Some(json!({"eye.iris.colour": "x"}))),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_settings_file_keeps_a_bounded_number_of_keys() {
        let e = env();
        let many: serde_json::Map<String, Value> = (0..MAX_PREFS)
            .map(|i| (format!("eye.l{i}.layout"), json!("grid")))
            .collect();
        let (st, _, _) = raw(&e, req("POST", "/api/prefs", Some(Value::Object(many)))).await;
        assert_eq!(st, StatusCode::OK);
        // one more is refused, a known one still changes
        let (st, _, _) = raw(
            &e,
            req("POST", "/api/prefs", Some(json!({"eye.theme": "dark"}))),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, _, _) = raw(
            &e,
            req(
                "POST",
                "/api/prefs",
                Some(json!({"eye.l0.layout": "lanes"})),
            ),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
    }
}
