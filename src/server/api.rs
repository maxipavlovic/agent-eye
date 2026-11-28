//! What every handler shares: the error type, the database handle, id lookup and the hidden set.

use super::{AppState, Shared};
use crate::config::Config;
use crate::db::Db;
use anyhow::Result;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

// ---------- errors ----------

/// An error answer: a status and `{"error": message}`, plus `"code"` when a client can act on it. The
/// last part is the whole chain of causes of an error that was answered with its first sentence
/// only: it can name paths, so it goes to the log (see `log_chains`), never to the client.
pub struct ApiError(StatusCode, String, Option<&'static str>, Option<String>);

impl ApiError {
    pub(super) fn bad(m: impl Into<String>) -> Self {
        ApiError(StatusCode::BAD_REQUEST, m.into(), None, None)
    }
    pub(super) fn not_found(m: impl Into<String>) -> Self {
        ApiError(StatusCode::NOT_FOUND, m.into(), None, None)
    }
    /// A request that is fine in itself but cannot be done now; `code` is stable for clients.
    pub(super) fn conflict(code: &'static str, m: impl Into<String>) -> Self {
        ApiError(StatusCode::CONFLICT, m.into(), Some(code), None)
    }
    /// Keep the causes of `e` for the log; the answer says only what this error says.
    pub(super) fn because(mut self, e: &anyhow::Error) -> Self {
        self.3 = Some(format!("{e:#}"));
        self
    }
}

impl From<anyhow::Error> for ApiError {
    /// The first sentence only: the rest of the chain names files and folders.
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), None, None).because(&e)
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("database: {e}"),
            None,
            None,
        )
    }
}

/// What `ApiError` leaves on a response for `log_chains`; it never reaches the client.
#[derive(Clone)]
struct Chain(String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match self.2 {
            Some(code) => json!({"error": self.1, "code": code}),
            None => json!({"error": self.1}),
        };
        let mut resp = (self.0, Json(body)).into_response();
        if let Some(chain) = self.3 {
            resp.extensions_mut().insert(Chain(chain));
        }
        resp
    }
}

/// Write the causes of an error that was answered with a short message to `logs/daemon.log`.
pub(super) async fn log_chains(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let what = format!("{} {}", req.method(), req.uri().path());
    let resp = next.run(req).await;
    if let Some(Chain(chain)) = resp.extensions().get::<Chain>() {
        crate::daemon::log(
            &st.paths,
            &format!("api {what}: {} {chain}", resp.status().as_u16()),
        );
    }
    resp
}

pub(super) type ApiResult = Result<Json<Value>, ApiError>;

pub(super) fn open_db(st: &AppState) -> Result<Db, ApiError> {
    Ok(Db::open(&st.paths.db_path())?)
}

/// Run work that blocks (a child process, the clipboard) off the async worker threads, so one
/// slow `osascript`, clipboard tool or agent start does not hold up every other request.
pub(super) async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::from(anyhow::anyhow!("background task failed: {e}")))?
}

pub(super) fn resolve(db: &Db, id: &str) -> Result<String, ApiError> {
    db.resolve_id(id)
        .map_err(|e| ApiError::not_found(format!("{e:#}")))
}

// ---------- handlers ----------

/// `?all=1` (or `true`) includes hidden sessions.
#[derive(Deserialize, Default)]
pub(super) struct AllQ {
    pub(super) all: Option<String>,
}

pub(super) fn wants_all(v: &Option<String>) -> bool {
    matches!(v.as_deref(), Some("1" | "true" | "yes"))
}

pub(super) fn hidden_set(
    db: &Db,
    cfg: &Config,
    all: bool,
) -> Result<std::collections::HashSet<String>> {
    if all {
        Ok(Default::default())
    } else {
        crate::hide::hidden_ids(db, cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn a_500_says_its_first_sentence_and_the_log_has_the_whole_chain() {
        use crate::config::Config;
        use crate::paths::Paths;
        use axum::routing::get;
        use axum::Router;
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let st = crate::server::new_state(paths, Config::default(), "t".repeat(48), 7420);
        let app = Router::new()
            .route(
                "/boom",
                get(|| async {
                    let e = anyhow::anyhow!("No such file or directory (os error 2)")
                        .context("opening /Users/someone/.agent-eye/eye.db")
                        .context("saving the thing");
                    Err::<Json<Value>, ApiError>(e.into())
                }),
            )
            .layer(axum::middleware::from_fn_with_state(st, log_chains));
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/boom")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v, json!({"error": "saving the thing"}));
        let log = std::fs::read_to_string(tmp.path().join("home/logs/daemon.log")).unwrap();
        assert!(
            log.contains("api GET /boom: 500 saving the thing: opening /Users/someone/.agent-eye/eye.db: No such file"),
            "{log}"
        );
    }

    #[tokio::test]
    async fn blocking_work_does_not_stall_the_runtime() {
        // this runtime has one thread: a sleep run inline would stop the ticker
        let ticks = Arc::new(AtomicUsize::new(0));
        let t = ticks.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(5)).await;
                t.fetch_add(1, Ordering::SeqCst);
            }
        });
        let r = blocking(|| {
            std::thread::sleep(Duration::from_millis(150)); // NOSONAR: runs on the blocking pool, which is the point of this test
            Ok(7)
        })
        .await;
        ticker.abort();
        assert_eq!(r.ok(), Some(7));
        assert!(ticks.load(Ordering::SeqCst) >= 5);
    }

    #[tokio::test]
    async fn blocking_work_passes_its_error_on() {
        let r: Result<(), ApiError> = blocking(|| Err(ApiError::bad("no"))).await;
        assert_eq!(r.err().map(|e| e.0), Some(StatusCode::BAD_REQUEST));
        let r: Result<(), ApiError> = blocking(|| panic!("boom")).await;
        assert_eq!(
            r.err().map(|e| e.0),
            Some(StatusCode::INTERNAL_SERVER_ERROR)
        );
    }
}
