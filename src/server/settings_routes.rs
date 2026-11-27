//! The settings the board's gear panel shows and changes: a whitelist of `config.toml` keys, each
//! with its type and what it may be. Everything else (folders, homes, prices, exclude lists, the
//! dangerous night switches) stays in `config.toml`; a key not listed here is a 400.

use super::api::{blocking, open_db, ApiError, ApiResult};
use super::{AppState, Shared};
use crate::agents::{self, origin};
use crate::config::{self, Config, PARALLEL_RANGE};
use crate::config_clock::BREAK_RANGE;
use crate::night::clock;
use crate::platform::idle;
use crate::quiet::{self, Found};
use crate::util::now_ms;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Map, Value};
use std::ops::RangeInclusive;

/// Every key the board may change, in the order the panel shows them.
const KEYS: &[&str] = &[
    "summary.enabled",
    "summary.model",
    "board.quiet",
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

/// A whole number inside `range` as the text to put after `key =`, or what the key takes.
fn int_in(key: &str, v: &Value, range: &RangeInclusive<u32>) -> Result<String, String> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
        .map(|n| n.to_string())
        .ok_or_else(|| {
            format!(
                "{key}: give a number from {} to {}",
                range.start(),
                range.end()
            )
        })
}

/// The ids and names of every agent eye knows, enabled or not.
fn known_agents(cfg: &Config) -> Vec<(String, String)> {
    agents::known(cfg)
        .iter()
        .map(|a| (a.id().to_string(), a.name().to_string()))
        .collect()
}

/// A list of words from a request, or what the key takes; a word listed twice is refused.
fn word_list(key: &str, v: &Value, what: &str) -> Result<Vec<String>, String> {
    let words: Vec<String> = v
        .as_array()
        .and_then(|a| a.iter().map(|w| w.as_str().map(String::from)).collect())
        .ok_or_else(|| format!("{key}: give {what}"))?;
    match words
        .iter()
        .enumerate()
        .find(|(n, w)| words[..*n].contains(w))
    {
        Some((_, w)) => Err(format!("{key}: '{w}' is listed twice")),
        None => Ok(words),
    }
}

/// One key as the panel gets it: its type, what it may be, and the value now. `restart` says the
/// daemon reads the key only when it starts. `summary.model` also names the agent that writes
/// summaries, its own default model and the names it takes, so the board can offer them.
/// `board.quiet` offers the program origins `found` on this machine, with their counts.
fn describe(cfg: &Config, key: &str, found: &[Found]) -> Value {
    match key {
        "summary.enabled" => json!({"type": "bool", "value": cfg.summary.enabled, "restart": true}),
        "summary.model" => {
            let enabled = agents::enabled(cfg);
            let agent = enabled.summary_agent(cfg);
            json!({
                "type": "text", "max": MODEL_MAX, "value": cfg.summary.model, "restart": true,
                "agent": agent.map(|a| a.name()),
                "default": agent.map_or("", |a| a.default_summary_model()),
                "choices": agent.map(|a| agents::summary_choices(a.as_ref())).unwrap_or_default(),
            })
        }
        "board.quiet" => {
            let choices: Vec<Value> = found
                .iter()
                .map(|f| {
                    json!({
                        "id": f.origin, "name": origin::chip(f.origin), "about": origin::started_by(f.origin),
                        "sessions": f.sessions, "needs_you": f.needs_you,
                    })
                })
                .collect();
            json!({"type": "origins", "choices": choices, "value": cfg.board.quiet})
        }
        "night.enabled" => json!({"type": "bool", "value": cfg.night.enabled}),
        "night.window" => {
            let w = clock::Window::parse(&cfg.night.window).unwrap_or_default();
            json!({"type": "window", "value": w.text()})
        }
        "night.parallel" => {
            json!({"type": "int", "min": PARALLEL_RANGE.start(), "max": PARALLEL_RANGE.end(), "value": cfg.night.parallel})
        }
        "night.isolation" => {
            json!({"type": "choice", "choices": ["worktree", "copy"], "value": cfg.night.isolation})
        }
        "remind.notify" => json!({"type": "bool", "value": cfg.remind.notify}),
        // the daemon's scan records turns by the setting it started with
        "clock.enabled" => json!({"type": "bool", "value": cfg.clock.enabled, "restart": true}),
        "clock.llm" => {
            json!({"type": "bool", "value": cfg.clock.llm.unwrap_or(cfg.summary.enabled)})
        }
        "clock.hints" => json!({"type": "bool", "value": cfg.clock.hints}),
        "clock.break_minutes" => {
            json!({"type": "int", "min": BREAK_RANGE.start(), "max": BREAK_RANGE.end(), "value": cfg.clock.break_minutes})
        }
        "clock.idle" => json!({
            "type": "bool", "value": cfg.clock.idle,
            "available": idle::idle_ms(&idle::RealSys).is_some(),
        }),
        "agents.enabled" => {
            let choices: Vec<Value> = known_agents(cfg)
                .into_iter()
                .map(|(id, name)| json!({"id": id, "name": name}))
                .collect();
            json!({"type": "agents", "choices": choices, "value": cfg.enabled_agent_ids(), "restart": true})
        }
        _ => unreachable!("{key} is in KEYS without being described"),
    }
}

fn all(cfg: &Config, found: &[Found]) -> Value {
    let map: Map<String, Value> = KEYS
        .iter()
        .map(|k| (k.to_string(), describe(cfg, k, found)))
        .collect();
    json!({"settings": map})
}

/// Every setting as the file says now (read again, so a change made elsewhere shows at once), with
/// the program origins this machine has.
fn settings_now(st: &AppState) -> Result<Value, ApiError> {
    let cfg = st.fresh_cfg();
    let found = quiet::found(&open_db(st)?, &cfg, now_ms())?;
    Ok(all(&cfg, &found))
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
        "night.parallel" => int_in(key, v, &PARALLEL_RANGE),
        "clock.break_minutes" => int_in(key, v, &BREAK_RANGE),
        "night.isolation" => match v.as_str() {
            Some(m @ ("worktree" | "copy")) => Ok(format!("{m:?}")),
            _ => Err(want("worktree or copy")),
        },
        "agents.enabled" => {
            let known = known_agents(cfg);
            let ids = word_list(key, v, "a list of agent ids")?;
            if let Some(id) = ids.iter().find(|id| !known.iter().any(|(k, _)| k == *id)) {
                return Err(format!("{key}: '{id}' is not an agent eye knows"));
            }
            Ok(config::toml_list(&ids))
        }
        "board.quiet" => {
            let words = word_list(key, v, "a list of origins")?;
            let origins = quiet::canonical(&words).map_err(|e| format!("{key}: {e}"))?;
            Ok(config::toml_list(&origins))
        }
        _ => unreachable!("{key} is in KEYS without being checked"),
    }
}

pub(super) async fn api_settings(State(st): State<Shared>) -> ApiResult {
    blocking(move || Ok(Json(settings_now(&st)?))).await
}

/// Writes the given keys into `config.toml` in one go, keeping its comments and other keys. Every
/// value is checked first: one bad key or value is a 400 and nothing is written.
pub(super) async fn api_settings_set(State(st): State<Shared>, Json(b): Json<Value>) -> ApiResult {
    let given = b
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| ApiError::bad("send an object with the settings to change"))?;
    let cfg = st.fresh_cfg();
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
        .map_err(|e| {
            // a file that does not parse or a lock held elsewhere is for the user to fix; a
            // disk that cannot be written is the server's trouble
            if e.chain().any(|c| c.is::<std::io::Error>()) {
                ApiError::from(e.context("saving settings"))
            } else {
                ApiError::bad(format!("saving settings: {e:#}"))
            }
        })?;
    blocking(move || Ok(Json(settings_now(&st)?))).await
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

    /// Does `key` stand in `text` as a whole word?
    fn has_word(text: &str, key: &str) -> bool {
        let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        text.match_indices(key).any(|(i, _)| {
            !is_word(text[..i].chars().next_back())
                && !is_word(text[i + key.len()..].chars().next())
        })
    }

    #[test]
    fn every_setting_and_its_fields_are_in_the_api_docs_the_config_docs_and_the_types() {
        let api = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/api.md"));
        let config = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/config.md"));
        let types = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/face/src/api/types.ts"
        ));
        // the route's own row of api.md, and the settings types of types.ts
        let row = api
            .lines()
            .find(|l| l.starts_with("| `GET /api/settings`"))
            .unwrap();
        let from = types.find("One setting of GET /api/settings").unwrap();
        let typed = &types[from..types[from..].find("SettingsInput").unwrap() + from];
        let cfg = crate::config::Config::default();
        // a machine with a program's sessions, so `board.quiet` has a choice to describe
        let found = [crate::quiet::Found {
            origin: "app",
            sessions: 5,
            needs_you: 2,
        }];
        for key in super::KEYS {
            assert!(
                row.contains(&format!("`{key}`")),
                "{key} is not in docs/api.md"
            );
            assert!(
                typed.contains(&format!("'{key}'")),
                "{key} is not in types.ts"
            );
            // config.md names a key under its table: `[clock]` and then `break_minutes`
            let (table, name) = key.split_once('.').unwrap();
            assert!(
                config.contains(&format!("[{table}]")) && config.contains(name),
                "{key} is not in docs/config.md"
            );
            let described = super::describe(&cfg, key, &found);
            let mut fields: Vec<&String> = described.as_object().unwrap().keys().collect();
            // and the fields of a choice that is an object (`agents.enabled`, `board.quiet`)
            if let Some(choice) = described["choices"].get(0).and_then(Value::as_object) {
                fields.extend(choice.keys());
            }
            for field in fields {
                assert!(has_word(row, field), "{key}: {field} is not in docs/api.md");
                assert!(has_word(typed, field), "{key}: {field} is not in types.ts");
            }
        }
    }

    #[tokio::test]
    async fn the_model_names_the_summary_agent_its_default_and_the_names_it_takes() {
        let e = env();
        let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(
            v["settings"]["summary.model"],
            json!({
                "type": "text", "max": 64, "value": "", "restart": true,
                "agent": "Claude Code", "default": "haiku", "choices": ["haiku", "sonnet", "opus"],
            })
        );
        let known = crate::agents::known(&crate::config::Config::default());
        let default_of = |id: &str| known.get(id).unwrap().default_summary_model();
        // `[summary] agent` when the file names one, else the first enabled agent; an agent that
        // lists no more names offers only its default
        for (file, agent, default) in [
            (
                "[agents]\nenabled = [\"claude\", \"codex\"]\n\n[summary]\nagent = \"codex\"\n",
                "Codex CLI",
                default_of("codex"),
            ),
            (
                "[agents]\nenabled = [\"agy\", \"claude\"]\n",
                "Antigravity CLI",
                default_of("agy"),
            ),
            ("[agents]\nenabled = [\"opencode\"]\n", "OpenCode", "free"),
        ] {
            std::fs::write(config_path(&e), file).unwrap();
            let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
            let m = &v["settings"]["summary.model"];
            assert_eq!(m["agent"], agent, "{file}");
            assert_eq!(m["default"], default, "{file}");
            assert_eq!(m["choices"], json!([default]), "{file}");
        }
        // no agent writes summaries: nothing to name or offer
        std::fs::write(config_path(&e), "[agents]\nenabled = []\n").unwrap();
        let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
        let m = &v["settings"]["summary.model"];
        assert_eq!(
            (&m["agent"], &m["default"], &m["choices"]),
            (&Value::Null, &json!(""), &json!([]))
        );
    }

    #[tokio::test]
    async fn every_model_the_board_offers_is_one_a_post_takes() {
        let e = env();
        for agent in crate::agents::known(&crate::config::Config::default()).iter() {
            for model in crate::agents::summary_choices(agent.as_ref()) {
                let (st, v) = post(&e, json!({"summary.model": model})).await;
                assert_eq!(st, StatusCode::OK, "{}: {model}: {v}", agent.id());
                assert_eq!(v["settings"]["summary.model"]["value"], model);
            }
        }
    }

    #[tokio::test]
    async fn the_defaults_come_with_their_types_and_allowed_values() {
        let e = env();
        let (st, v) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(st, StatusCode::OK);
        let s = &v["settings"];
        assert_eq!(s.as_object().unwrap().len(), 14);
        assert_eq!(
            s["board.quiet"],
            json!({"type": "origins", "choices": [], "value": []})
        );
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
            json!({"board.quiet": ["terminal"]}),
            json!({"board.quiet": ["app", "app"]}),
            json!({"board.quiet": ["app", "App"]}),
            json!({"board.quiet": "app"}),
            json!({"board.quiet": [1]}),
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
    async fn quiet_offers_the_program_origins_found_here_and_the_board_follows_it_at_once() {
        let e = env();
        let now = crate::util::now_ms();
        for (id, origin, ai) in [
            ("app1", "app", "needs_you"),
            ("app2", "app", "paused"),
            ("term", "terminal", "needs_you"),
        ] {
            add_session(&e.db, id, id, Some(ai), 3, now - 60_000);
            // a folder that is there, so the session can be resumed
            let cwd = e._tmp.path().to_string_lossy().to_string();
            e.db.conn
                .execute(
                    "UPDATE session SET origin=?2, cwd=?3 WHERE id=?1",
                    rusqlite::params![id, origin, cwd],
                )
                .unwrap();
        }
        let app = json!({
            "id": "app", "name": "App", "about": "started by an app through an agent's SDK",
            "sessions": 2, "needs_you": 1,
        });
        let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
        assert_eq!(
            v["settings"]["board.quiet"],
            json!({"type": "origins", "choices": [app], "value": []})
        );
        let needs_you = |b: &Value| card_ids(b, "needs_you");
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert!(needs_you(&b).contains(&"app1".to_string()));

        // the word the board shows works too, and the file gets the origin
        let (st, v) = post(&e, json!({"board.quiet": ["App"]})).await;
        assert_eq!(st, StatusCode::OK, "{v}");
        // the count still says what it clears
        assert_eq!(
            v["settings"]["board.quiet"],
            json!({"type": "origins", "choices": [app], "value": ["app"]})
        );
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(text.contains("quiet = [\"app\"]"), "{text}");
        // the board and a card follow the file without a restart
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert_eq!(needs_you(&b), ["term"]);
        assert!(!card_ids(&b, "paused").contains(&"app1".to_string()));
        let (_, c) = call(&e, req("GET", "/api/cards/app1", None)).await;
        assert_eq!(c["column"], "archive");
        // still listed and resumable, with its origin shown
        assert_eq!(
            (&c["resumable"], &c["origin"]),
            (&json!(true), &json!("app"))
        );

        // an origin set to quiet with no sessions keeps its switch
        let (_, v) = post(&e, json!({"board.quiet": ["app", "headless"]})).await;
        let ids: Vec<&str> = v["settings"]["board.quiet"]["choices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["app", "headless"]);
        assert_eq!(v["settings"]["board.quiet"]["choices"][1]["sessions"], 0);
        post(&e, json!({"board.quiet": []})).await;
        let (_, b) = call(&e, req("GET", "/api/board", None)).await;
        assert!(needs_you(&b).contains(&"app1".to_string()));
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
    async fn a_file_that_cannot_be_written_is_a_500_not_a_400() {
        let e = env();
        std::fs::create_dir(config_path(&e)).unwrap();
        let (st, v) = post(&e, json!({"remind.notify": true})).await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
        assert!(v["error"].as_str().unwrap().contains("saving settings"));
    }

    #[tokio::test]
    async fn the_ranges_come_from_the_rules_the_config_and_the_cli_use() {
        let e = env();
        let (_, v) = call(&e, req("GET", "/api/settings", None)).await;
        let s = &v["settings"];
        assert_eq!(
            s["night.parallel"]["min"],
            *crate::config::PARALLEL_RANGE.start()
        );
        assert_eq!(
            s["night.parallel"]["max"],
            *crate::config::PARALLEL_RANGE.end()
        );
        assert_eq!(
            s["clock.break_minutes"]["min"],
            *crate::config_clock::BREAK_RANGE.start()
        );
        assert_eq!(
            s["clock.break_minutes"]["max"],
            *crate::config_clock::BREAK_RANGE.end()
        );
        let (st, v) = post(&e, json!({"clock.break_minutes": 241})).await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("from 10 to 240"));
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
