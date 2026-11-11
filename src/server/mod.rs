//! The board server: JSON over HTTP on 127.0.0.1, plus the embedded face. See `docs/api.md`.

mod agent_routes;
mod api;
mod board;
mod card;
mod cards;
mod face;
mod guard;
mod idea_routes;
mod prefs;
mod remind_routes;
mod run;
#[cfg(test)]
mod test_support;

pub use board::board_json;
pub use card::{card_json, CardCtx};
pub use guard::ensure_token;
pub use run::{board_url, serve, shutdown_signal};

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
        .route("/api/cards/:id", get(cards::api_card))
        .route("/api/cards/:id/column", post(cards::api_move))
        .route("/api/cards/:id/hidden", post(cards::api_hidden))
        .route("/api/cards/:id/remind", post(remind_routes::api_remind))
        .route("/api/cards/:id/resume", post(cards::api_resume))
        .route("/api/cards/:id/conversation", get(cards::api_conversation))
        .route("/api/search", get(cards::api_search))
        .route("/api/agents", get(agent_routes::api_agents))
        .route(
            "/api/ideas",
            get(idea_routes::api_ideas).post(idea_routes::api_idea_add),
        )
        .route("/api/ideas/:id", post(idea_routes::api_idea_edit))
        .route("/api/ideas/:id/start", post(idea_routes::api_idea_start))
        .route("/api/folder", get(idea_routes::api_folder))
        .route("/api/events", get(run::api_events))
        .fallback(get(face::face_asset))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(state.clone(), guard::guard))
        .with_state(state)
}
