//! Running the server: the change watcher behind the event stream, the listener, shutdown.

use super::guard::ensure_token;
use super::{new_state, router, Shared};
use crate::config::Config;
use crate::db::Db;
use crate::paths::Paths;
use anyhow::{Context, Result};
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub(super) async fn api_events(
    State(st): State<Shared>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = st.changes.subscribe();
    let stream =
        BroadcastStream::new(rx).map(|_| Ok(Event::default().event("board-changed").data("1")));
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------- run ----------

/// Poll `PRAGMA data_version` and tell SSE clients when another connection changed the db.
pub(super) fn spawn_watcher(st: Shared) {
    tokio::spawn(async move {
        let mut last: Option<i64> = None;
        let mut db: Option<Db> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if db.is_none() {
                db = Db::open(&st.paths.db_path()).ok();
            }
            let Some(d) = db.as_ref() else { continue };
            match d.data_version() {
                Ok(v) => {
                    if last.is_some() && last != Some(v) {
                        let _ = st.changes.send(());
                    }
                    last = Some(v);
                }
                Err(_) => db = None,
            }
        }
    });
}

/// Resolves on Ctrl-C or SIGTERM.
pub async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

/// The link that opens the board and logs the browser in.
pub fn board_url(port: u16, token: &str) -> String {
    format!("http://127.0.0.1:{port}/?t={token}")
}

/// Serve the board until Ctrl-C. `on_ready` gets the URL once the port is bound.
pub fn serve(
    paths: Paths,
    cfg: Config,
    port: u16,
    on_ready: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    let token = ensure_token(&paths)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .with_context(|| format!("port {port} is busy; try --port"))?;
        let port = listener.local_addr()?.port();
        let state = new_state(paths, cfg, token.clone(), port);
        spawn_watcher(state.clone());
        on_ready(&board_url(port, &token))?;
        axum::serve(listener, router(state))
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        Ok(())
    })
}
