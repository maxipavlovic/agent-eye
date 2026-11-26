//! The settings the board's gear panel shows and changes: a whitelist of `config.toml` keys, each
//! with its type and what it may be. Everything else (folders, homes, prices, exclude lists, the
//! dangerous night switches) stays in the file and `eye config`; a key not listed here is a 400.

use super::api::{ApiError, ApiResult};
use super::{AppState, Shared};
use crate::agents;
use crate::config::{self, Config};
use crate::night::clock;
use crate::platform::idle;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Map, Value};

/// Every key the board may change, in the order the panel shows them.
const KEYS: &[&str] = &[
    "summary.enabled",
    "summary.model",
    "night.enabled",
    "night.window",
    "night.parallel",
    "night.isolation",
    "remind.notify",
    "agents.enabled",
    "clock.enabled",
    "clock.llm",
    "clock.hints",
    "clock.break_minutes",
    "clock.idle",
];

/// The longest model name accepted: they are short names such as `haiku` or `provider/model`.
const MODEL_MAX: usize = 64;

/// The settings now: the file is read again, so a change made elsewhere shows at once.
fn fresh_cfg(st: &AppState) -> Config {
    std::fs::read_to_string(st.paths.config_path())
        .ok()
        .and_then(|t| Config::parse(&t).ok())
        .unwrap_or_else(|| st.cfg.clone())
}

/// The ids and names of every agent eye knows, enabled or not.
fn known_agents(cfg: &Config) -> Vec<(String, String)> {
    agents::known(cfg)
        .iter()
        .map(|a| (a.id().to_string(), a.name().to_string()))
        .collect()
}

/// One key as the panel gets it: its type, what it may be, and the value now. `restart` says the
/// daemon reads the key only when it starts.
fn describe(cfg: &Config, key: &str) -> Value {
    match key {
        "summary.enabled" => json!({"type": "bool", "value": cfg.summary.enabled, "restart": true}),
        "summary.model" => {
            json!({"type": "text", "max": MODEL_MAX, "value": cfg.summary.model, "restart": true})
        }
        "night.enabled" => json!({"type": "bool", "value": cfg.night.enabled}),
        "night.window" => {
            let w = clock::Window::parse(&cfg.night.window).unwrap_or_default();
            json!({"type": "window", "value": w.text()})
        }
        "night.parallel" => {
            json!({"type": "int", "min": 1, "max": 8, "value": cfg.night.parallel})
        }
        "night.isolation" => {
            json!({"type": "choice", "choices": ["worktree", "copy"], "value": cfg.night.isolation})
        }
        "remind.notify" => json!({"type": "bool", "value": cfg.remind.notify}),
        "clock.enabled" => json!({"type": "bool", "value": cfg.clock.enabled}),
        "clock.llm" => {
            json!({"type": "bool", "value": cfg.clock.llm.unwrap_or(cfg.summary.enabled)})
        }
        "clock.hints" => json!({"type": "bool", "value": cfg.clock.hints}),
        "clock.break_minutes" => {
            json!({"type": "int", "min": 10, "max": 240, "value": cfg.clock.break_minutes})
        }
        "clock.idle" => json!({
            "type": "bool", "value": cfg.clock.idle,
            "available": idle::idle_ms(&idle::RealSys).is_some(),
        }),
        _ => {
            let choices: Vec<Value> = known_agents(cfg)
                .into_iter()
                .map(|(id, name)| json!({"id": id, "name": name}))
                .collect();
            json!({"type": "agents", "choices": choices, "value": cfg.enabled_agent_ids(), "restart": true})
        }
    }
}

fn all(cfg: &Config) -> Value {
    let map: Map<String, Value> = KEYS
        .iter()
        .map(|k| (k.to_string(), describe(cfg, k)))
        .collect();
    json!({"settings": map})
}

/// A value from a request as the text to put after `key =` in the file, or why it is refused.
fn toml_value(cfg: &Config, key: &str, v: &Value) -> Result<String, String> {
    let want = |what: &str| format!("{key}: give {what}");
    match key {
        "summary.enabled" | "night.enabled" | "remind.notify" | "clock.enabled" | "clock.llm"
        | "clock.hints" | "clock.idle" => v
            .as_bool()
            .map(|b| b.to_string())
            .ok_or_else(|| want("true or false")),
        "summary.model" => {
            let m = v.as_str().ok_or_else(|| want("a model name"))?.trim();
            let plain = |c: char| c.is_ascii_alphanumeric() || "._:/@-".contains(c);
            if m.len() > MODEL_MAX || !m.chars().all(plain) {
                return Err(format!(
                    "{key}: a model name is up to {MODEL_MAX} letters, digits and . _ : / @ -"
                ));
            }
            Ok(format!("{m:?}"))
        }
        "night.window" => {
            let w = clock::Window::parse(
                v.as_str()
                    .ok_or_else(|| want("a window like 01:00-07:00"))?,
            )?;
            Ok(format!("{:?}", w.text()))
        }
        "night.parallel" => v
            .as_u64()
            .filter(|n| (1..=8).contains(n))
            .map(|n| n.to_string())
            .ok_or_else(|| want("a number from 1 to 8")),
        "clock.break_minutes" => v
            .as_u64()
            .filter(|n| (10..=240).contains(n))
            .map(|n| n.to_string())
            .ok_or_else(|| want("a number from 10 to 240")),
        "night.isolation" => match v.as_str() {
            Some(m @ ("worktree" | "copy")) => Ok(format!("{m:?}")),
            _ => Err(want("worktree or copy")),
        },
        _ => {
            let known = known_agents(cfg);
            let ids: Vec<&str> = v
                .as_array()
                .ok_or_else(|| want("a list of agent ids"))?
                .iter()
                .map(|i| i.as_str().ok_or_else(|| want("a list of agent ids")))
                .collect::<Result<_, _>>()?;
            for (n, id) in ids.iter().enumerate() {
                if !known.iter().any(|(k, _)| k == id) {
                    return Err(format!("{key}: '{id}' is not an agent eye knows"));
                }
                if ids[..n].contains(id) {
                    return Err(format!("{key}: '{id}' is listed twice"));
                }
            }
            let ids: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
            Ok(config::agents_list(&ids))
        }
    }
}

pub(super) async fn api_settings(State(st): State<Shared>) -> ApiResult {
    Ok(Json(all(&fresh_cfg(&st))))
}

/// Writes the given keys into `config.toml` in one go, keeping its comments and other keys. Every
/// value is checked first: one bad key or value is a 400 and nothing is written.
pub(super) async fn api_settings_set(State(st): State<Shared>, Json(b): Json<Value>) -> ApiResult {
    let given = b
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| ApiError::bad("send an object with the settings to change"))?;
    let cfg = fresh_cfg(&st);
    let mut edits = Vec::new();
    for (key, v) in given {
        let key = KEYS
            .iter()
            .find(|k| **k == key)
            .ok_or_else(|| ApiError::bad(format!("{key}: not a setting the board changes")))?;
        let (table, name) = key.split_once('.').unwrap_or_default();
        edits.push((
            table,
            name,
            toml_value(&cfg, key, v).map_err(ApiError::bad)?,
        ));
    }
    let tables: Vec<String> = edits.iter().map(|(t, _, _)| format!("[{t}]")).collect();
    let keys: Vec<(&str, &str, String)> = edits
        .into_iter()
        .zip(&tables)
        .map(|((_, k, v), t)| (t.as_str(), k, v))
        .collect();
    st.paths
        .ensure_home()
        .and_then(|()| config::write_keys(&st.paths.config_path(), &keys))
        .map_err(|e| ApiError::bad(format!("saving settings: {e:#}")))?;
    Ok(Json(all(&fresh_cfg(&st))))
}

#[cfg(test)]
mod tests {
    use crate::server::test_support::*;
    use axum::http::StatusCode;
    use serde_json::{json, Value};

    fn config_path(e: &Env) -> std::path::PathBuf {
        e._tmp.path().join("home").join("config.toml")
    }

    async fn post(e: &Env, body: Value) -> (StatusCode, Value) {
        call(e, req("POST", "/api/settings", Some(body))).await
    }

    #[tokio::test]
    async fn the_defaults_come_with_their_types_and_allowed_values() {
        let e = env();
        let (st, v) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(st, StatusCode::OK);
        let s = &v["settings"];
        assert_eq!(s.as_object().unwrap().len(), 13);
        assert_eq!(
            s["summary.enabled"],
            json!({"type": "bool", "value": true, "restart": true})
        );
        assert_eq!(
            s["night.window"],
            json!({"type": "window", "value": "01:00-07:00"})
        );
        assert_eq!(s["night.parallel"]["max"], 8);
        assert_eq!(s["night.isolation"]["choices"], json!(["worktree", "copy"]));
        assert_eq!(s["remind.notify"]["value"], false);
        assert_eq!(s["agents.enabled"]["value"], json!(["claude"]));
        assert_eq!(s["clock.enabled"]["value"], true);
        assert_eq!(s["clock.hints"]["value"], false);
        assert_eq!(s["clock.idle"]["value"], false);
        assert!(s["clock.idle"]["available"].is_boolean());
        assert_eq!(
            s["clock.break_minutes"],
            json!({"type": "int", "min": 10, "max": 240, "value": 45})
        );
        let ids: Vec<&str> = s["agents.enabled"]["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["claude", "codex", "agy", "opencode"]);
    }

    #[tokio::test]
    async fn a_post_changes_several_keys_at_once_and_keeps_the_config() {
        let e = env();
        std::fs::write(
            config_path(&e),
            "[board]\nport = 7500 # mine\n\n[summary]\nmodel = \"haiku\" # cheap\nsomething_new = 1\n",
        )
        .unwrap();
        let (st, v) = post(
            &e,
            json!({
                "summary.model": "sonnet",
                "summary.enabled": false,
                "night.window": "23:00-5:00",
                "night.parallel": 3,
                "night.isolation": "copy",
                "remind.notify": true,
                "agents.enabled": ["codex", "claude"],
            }),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let s = &v["settings"];
        assert_eq!(s["summary.model"]["value"], "sonnet");
        assert_eq!(s["summary.enabled"]["value"], false);
        assert_eq!(s["night.window"]["value"], "23:00-05:00");
        assert_eq!(s["night.parallel"]["value"], 3);
        assert_eq!(s["night.isolation"]["value"], "copy");
        assert_eq!(s["remind.notify"]["value"], true);
        assert_eq!(s["agents.enabled"]["value"], json!(["codex", "claude"]));
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(text.contains("port = 7500 # mine"), "{text}");
        assert!(text.contains("model = \"sonnet\"    # cheap"), "{text}");
        assert!(text.contains("something_new = 1"), "{text}");
        let (_, again) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(again, v);
    }

    #[tokio::test]
    async fn the_clock_keys_are_written_and_llm_follows_summary_until_set() {
        let e = env();
        let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(v["settings"]["clock.llm"]["value"], true);
        let (st, v) = post(&e, json!({"summary.enabled": false})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["settings"]["clock.llm"]["value"], false);
        let (st, v) = post(
            &e,
            json!({"clock.llm": true, "clock.hints": true, "clock.break_minutes": 30, "clock.idle": true, "clock.enabled": false}),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        let s = &v["settings"];
        assert_eq!(s["clock.llm"]["value"], true);
        assert_eq!(s["clock.hints"]["value"], true);
        assert_eq!(s["clock.break_minutes"]["value"], 30);
        assert_eq!(s["clock.idle"]["value"], true);
        assert_eq!(s["clock.enabled"]["value"], false);
        for bad in [
            json!({"clock.break_minutes": 5}),
            json!({"clock.break_minutes": 241}),
            json!({"clock.hints": "on"}),
        ] {
            let (st, _) = post(&e, bad).await;
            assert_eq!(st, StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn the_remind_route_and_this_one_agree() {
        let e = env();
        post(&e, json!({"remind.notify": true})).await;
        let (_, v) = call(&e, req("GET", "/api/remind/settings", None)).await;
        assert_eq!(v["notify"], true);
    }

    #[tokio::test]
    async fn an_unknown_key_or_a_bad_value_is_a_400_and_writes_nothing() {
        let e = env();
        let bad = [
            json!({"board.port": 1}),
            json!({"night.allow_unsandboxed": true}),
            json!({"summary.enabled": "yes"}),
            json!({"summary.model": "a b"}),
            json!({"summary.model": "x".repeat(65)}),
            json!({"night.window": "7am"}),
            json!({"night.window": "01:00-01:00"}),
            json!({"night.parallel": 0}),
            json!({"night.parallel": 9}),
            json!({"night.parallel": "2"}),
            json!({"night.isolation": "none"}),
            json!({"agents.enabled": ["nobody"]}),
            json!({"agents.enabled": ["claude", "claude"]}),
            json!({"agents.enabled": "claude"}),
            json!({}),
            json!([1]),
            // one good key does not save a request that has a bad one
            json!({"remind.notify": true, "night.parallel": 99}),
        ];
        for body in bad {
            let (st, v) = post(&e, body.clone()).await;
            assert_eq!(st, StatusCode::BAD_REQUEST, "{body}: {v}");
            assert!(v["error"].is_string());
        }
        assert!(!config_path(&e).exists(), "nothing was written");
    }

    #[tokio::test]
    async fn an_empty_agent_list_is_allowed_as_in_the_config_file() {
        let e = env();
        let (st, v) = post(&e, json!({"agents.enabled": []})).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["settings"]["agents.enabled"]["value"], json!([]));
    }

    #[tokio::test]
    async fn a_post_with_a_config_that_does_not_parse_is_a_400_and_writes_nothing() {
        let e = env();
        let path = config_path(&e);
        std::fs::write(&path, "[remind\nnotify = false\n").unwrap();
        let (st, v) = post(&e, json!({"remind.notify": true})).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("does not parse"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[remind\nnotify = false\n"
        );
    }

    #[tokio::test]
    async fn the_route_keeps_the_token_host_and_origin_rules() {
        let e = env();
        let mut r = req("GET", "/api/settings", None);
        r.headers_mut().remove("x-eye-token");
        assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED);
        let mut r = req("GET", "/api/settings", None);
        r.headers_mut()
            .insert("host", "evil.example:7420".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
        let mut r = req(
            "POST",
            "/api/settings",
            Some(json!({"remind.notify": true})),
        );
        r.headers_mut()
            .insert("origin", "http://evil.example".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
        assert!(!config_path(&e).exists());
    }
}
