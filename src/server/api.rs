//! What every handler shares: the error type, the database handle, id lookup and the hidden set.

use super::AppState;
use crate::config::Config;
use crate::db::Db;
use anyhow::Result;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

// ---------- errors ----------

/// An error answer: a status and `{"error": message}`.
pub struct ApiError(StatusCode, String);

impl ApiError {
    pub(super) fn bad(m: impl Into<String>) -> Self {
        ApiError(StatusCode::BAD_REQUEST, m.into())
    }
    pub(super) fn not_found(m: impl Into<String>) -> Self {
        ApiError(StatusCode::NOT_FOUND, m.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("database: {e}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
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
