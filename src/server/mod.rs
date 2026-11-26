//! The board server: JSON over HTTP on 127.0.0.1, plus the embedded face. See `docs/api.md`.

mod agent_routes;
mod api;
mod board;
mod card;
mod cards;
mod clock_routes;
mod face;
mod guard;
mod idea_routes;
mod loose_routes;
mod night_routes;
mod prefs;
mod remind_routes;
mod remind_settings;
mod run;
mod settings_routes;
#[cfg(test)]
mod test_support;
mod usage_routes;

pub use board::board_json;
pub use card::{card_json, CardCtx};
pub use guard::ensure_token;
pub use run::{ask_shutdown, board_url, serve, shutdown_signal};

use crate::agents::{self, Agents};
use crate::config::Config;
use crate::paths::Paths;
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct AppState {
    pub paths: Paths,
    pub cfg: Config,
    /// The enabled agents, in order; the first is the default.
    pub agents: Agents,
    pub token: String,
    pub port: u16,
    pub changes: broadcast::Sender<()>,
    /// `EYE_FACE_DIR`: serve the face from this folder instead of the embedded copy.
    pub face_dir: Option<std::path::PathBuf>,
}

impl AppState {
    fn card_ctx(&self) -> CardCtx<'_> {
        CardCtx {
            user: &self.cfg.user,
            agents: &self.agents,
        }
    }
}

type Shared = Arc<AppState>;

pub fn new_state(paths: Paths, cfg: Config, token: String, port: u16) -> Shared {
    let face_dir = std::env::var_os("EYE_FACE_DIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from);
    new_state_with_face(paths, cfg, token, port, face_dir)
}

pub fn new_state_with_face(
    paths: Paths,
    cfg: Config,
    token: String,
    port: u16,
    face_dir: Option<std::path::PathBuf>,
) -> Shared {
    let (changes, _) = broadcast::channel(16);
    Arc::new(AppState {
        paths,
        agents: agents::enabled(&cfg),
        cfg,
        token,
        port,
        changes,
        face_dir,
    })
}

pub fn router(state: Shared) -> Router {
    Router::new()
        .route("/", get(face::face_index))
        .route("/manifest.webmanifest", get(face::app_manifest))
        .route("/api/board", get(board::api_board))
        .route(
            "/api/prefs",
            get(prefs::api_prefs).post(prefs::api_prefs_set),
        )
        .route("/api/archive", get(board::api_archive))
        .route("/api/cards/{id}", get(cards::api_card))
        .route("/api/cards/{id}/column", post(cards::api_move))
        .route("/api/cards/{id}/hidden", post(cards::api_hidden))
        .route("/api/sessions/{id}/forget", post(cards::api_forget))
        .route("/api/cards/{id}/remind", post(remind_routes::api_remind))
        .route(
            "/api/remind/settings",
            get(remind_settings::api_remind_settings)
                .post(remind_settings::api_remind_settings_set),
        )
        .route(
            "/api/settings",
            get(settings_routes::api_settings).post(settings_routes::api_settings_set),
        )
        .route("/api/cards/{id}/resume", post(cards::api_resume))
        .route("/api/cards/{id}/conversation", get(cards::api_conversation))
        .route("/api/sessions/{id}/export", get(cards::api_export))
        .route("/api/search", get(cards::api_search))
        .route("/api/agents", get(agent_routes::api_agents))
        .route("/api/usage", get(usage_routes::api_usage))
        .route("/api/clock", get(clock_routes::api_clock))
        .route(
            "/api/ideas",
            get(idea_routes::api_ideas).post(idea_routes::api_idea_add),
        )
        .route("/api/ideas/{id}", post(idea_routes::api_idea_edit))
        .route("/api/ideas/{id}/start", post(idea_routes::api_idea_start))
        .route("/api/folder", get(idea_routes::api_folder))
        .route("/api/loose", get(loose_routes::api_loose))
        .route("/api/loose/stop", post(loose_routes::api_loose_stop))
        .route("/api/loose/remove", post(loose_routes::api_loose_remove))
        .route("/api/loose/ignore", post(loose_routes::api_loose_ignore))
        .route("/api/loose/send", post(loose_routes::api_loose_send))
        .route("/api/night", get(night_routes::api_night))
        .route(
            "/api/night/settings",
            post(night_routes::api_night_settings),
        )
        .route("/api/night/plan", post(night_routes::api_night_plan))
        .route(
            "/api/night/session/{id}",
            get(night_routes::api_night_of_session),
        )
        .route(
            "/api/night/{id}/cancel",
            post(night_routes::api_night_cancel),
        )
        .route("/api/night/{id}/limit", post(night_routes::api_night_limit))
        .route(
            "/api/night/{id}/discard",
            post(night_routes::api_night_discard),
        )
        .route("/api/night/{id}/open", post(night_routes::api_night_open))
        .route("/api/night/{id}/bring", post(night_routes::api_night_bring))
        .route("/api/events", get(run::api_events))
        .fallback(get(face::face_asset))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard::guard))
        .with_state(state)
}
