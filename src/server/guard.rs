//! Who may talk to the server: the token (header or cookie), the Host and Origin checks, and the
//! token file.

use super::Shared;
use crate::paths::Paths;
use crate::util::{hex, random_bytes};
use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::os::unix::fs::OpenOptionsExt;

/// Read `$EYE_HOME/board.token`, creating it (0600, 32 random bytes as hex) when missing.
pub fn ensure_token(paths: &Paths) -> Result<String> {
    let p = paths.home.join("board.token");
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            crate::files::make_private(&p);
            return Ok(t);
        }
    }
    let t = hex(&random_bytes(32)?);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&p)
        .with_context(|| format!("writing {}", p.display()))?;
    std::io::Write::write_all(&mut f, t.as_bytes())?;
    Ok(t)
}

// ---------- guard: Host, Origin, token ----------

pub(super) fn cookie_token(h: &HeaderMap) -> Option<String> {
    let raw = h.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find(|(k, _)| *k == "eye_token")
        .map(|(_, v)| v.to_string())
}

pub(super) fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

pub(super) fn query_param(q: Option<&str>, key: &str) -> Option<String> {
    q?.split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

pub(super) const PUBLIC_FILES: &[&str] = &["/favicon.svg", "/icon-192.png", "/icon-512.png"];

pub(super) async fn guard(State(st): State<Shared>, req: Request, next: Next) -> Response {
    // Only the loopback names with the board's own port: a page on another site that was pointed
    // at 127.0.0.1 (DNS rebinding) still sends its own name in Host, and is refused here.
    let hosts = [
        format!("127.0.0.1:{}", st.port),
        format!("localhost:{}", st.port),
        format!("[::1]:{}", st.port),
    ];
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !hosts.contains(&host) {
        return (StatusCode::FORBIDDEN, "bad Host header").into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        // Browsers always send Origin on a POST; a missing one is not a browser on this page.
        let o = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !hosts
            .iter()
            .any(|h| o.to_ascii_lowercase() == format!("http://{h}"))
        {
            return (StatusCode::FORBIDDEN, "missing or bad Origin header").into_response();
        }
    }
    if req.method() == Method::GET && req.uri().path() == "/" {
        if let Some(t) = query_param(req.uri().query(), "t") {
            if same(&t, &st.token) {
                // Keep the other query parameters (e.g. `?look=ink`); drop only the token.
                let rest: Vec<&str> = req
                    .uri()
                    .query()
                    .unwrap_or("")
                    .split('&')
                    .filter(|p| !p.is_empty() && !p.starts_with("t="))
                    .collect();
                let loc = if rest.is_empty() {
                    "/".to_string()
                } else {
                    format!("/?{}", rest.join("&"))
                };
                return Response::builder()
                    .status(StatusCode::SEE_OTHER)
                    .header(header::LOCATION, loc)
                    .header(
                        header::SET_COOKIE,
                        // A year, so the board installed as an app survives a browser restart.
                        format!(
                            "eye_token={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=31536000",
                            st.token
                        ),
                    )
                    .body(axum::body::Body::empty())
                    .unwrap_or_default();
            }
        }
    }
    // The app icons hold nothing private; browsers fetch them without the cookie.
    if req.method() == Method::GET && PUBLIC_FILES.contains(&req.uri().path()) {
        return next.run(req).await;
    }
    let ok = cookie_token(req.headers())
        .or_else(|| {
            req.headers()
                .get("x-eye-token")
                .and_then(|v| v.to_str().ok())
                .map(String::from)
        })
        .map(|t| same(&t, &st.token))
        .unwrap_or(false);
    if !ok {
        return (
            StatusCode::UNAUTHORIZED,
            format!(
                "missing token: open the URL printed by `{} board`",
                crate::pick::bin_name()
            ),
        )
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ideas;
    use crate::server::test_support::*;
    use crate::util::now_ms;
    use axum::body::Body;
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn face_routes_keep_the_guard() {
        let d = face_dir();
        let e = env_face(Some(d.path().to_path_buf()));
        for uri in ["/", "/assets/app-abc123.js", "/nothing"] {
            let r = Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:7420")
                .body(Body::empty())
                .unwrap();
            assert_eq!(raw(&e, r).await.0, StatusCode::UNAUTHORIZED, "{uri}");
            let r = Request::builder()
                .uri(uri)
                .header("host", "evil.example:7420")
                .header("x-eye-token", TOKEN)
                .body(Body::empty())
                .unwrap();
            assert_eq!(raw(&e, r).await.0, StatusCode::FORBIDDEN, "{uri}");
        }
        // Other query parameters survive the redirect; the token does not.
        let r = Request::builder()
            .uri(format!("/?t={TOKEN}&look=ink"))
            .header("host", "127.0.0.1:7420")
            .body(Body::empty())
            .unwrap();
        let (st, h, _) = raw(&e, r).await;
        assert_eq!(st, StatusCode::SEE_OTHER);
        assert_eq!(h[header::LOCATION], "/?look=ink");
        // The token link still sets the cookie and redirects.
        let r = Request::builder()
            .uri(format!("/?t={TOKEN}"))
            .header("host", "127.0.0.1:7420")
            .body(Body::empty())
            .unwrap();
        let (st, h, _) = raw(&e, r).await;
        assert_eq!(st, StatusCode::SEE_OTHER);
        assert!(h[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .starts_with("eye_token="));
    }

    #[tokio::test]
    async fn auth_flow() {
        let e = env();
        let no_token = Request::builder()
            .uri("/api/board")
            .header("host", "127.0.0.1:7420")
            .body(Body::empty())
            .unwrap();
        assert_eq!(call(&e, no_token).await.0, StatusCode::UNAUTHORIZED);

        let mut bad_host = req("GET", "/api/board", None);
        bad_host
            .headers_mut()
            .insert("host", "evil.example:7420".parse().unwrap());
        assert_eq!(call(&e, bad_host).await.0, StatusCode::FORBIDDEN);

        let wrong = Request::builder()
            .uri("/?t=nope")
            .header("host", "localhost:7420")
            .body(Body::empty())
            .unwrap();
        assert_eq!(call(&e, wrong).await.0, StatusCode::UNAUTHORIZED);

        let login = Request::builder()
            .uri(format!("/?t={TOKEN}"))
            .header("host", "localhost:7420")
            .body(Body::empty())
            .unwrap();
        let resp = e.app.clone().oneshot(login).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let cookie = resp.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));

        let with_cookie = Request::builder()
            .uri("/api/board")
            .header("host", "127.0.0.1:7420")
            .header("cookie", format!("a=b; eye_token={TOKEN}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(call(&e, with_cookie).await.0, StatusCode::OK);

        let mut bad_origin = req("POST", "/api/ideas", Some(json!({"title": "x"})));
        bad_origin
            .headers_mut()
            .insert("origin", "http://evil.example".parse().unwrap());
        assert_eq!(call(&e, bad_origin).await.0, StatusCode::FORBIDDEN);
        let mut ok_origin = req("POST", "/api/ideas", Some(json!({"title": "x"})));
        ok_origin
            .headers_mut()
            .insert("origin", "http://localhost:7420".parse().unwrap());
        assert_eq!(call(&e, ok_origin).await.0, StatusCode::OK);
    }

    #[test]
    fn token_file_is_private_and_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().join("h"),
        };
        paths.ensure_home().unwrap();
        let t1 = ensure_token(&paths).unwrap();
        assert_eq!(t1.len(), 64);
        assert_eq!(ensure_token(&paths).unwrap(), t1);
        let mode = std::fs::metadata(paths.home.join("board.token")).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn post_needs_origin_and_stays_dry() {
        let e = env();
        add_session(&e.db, "aaaaaaaa-1", "t", None, 1, now_ms());
        ideas::add(
            &e.db,
            &ideas::NewIdea {
                title: "i",
                notes: Some("n"),
                folder: Some("/tmp"),
                ..Default::default()
            },
        )
        .unwrap();
        for uri in ["/api/cards/aaaaaaaa-1/resume", "/api/ideas/1/start"] {
            let mut r = req("POST", uri, Some(json!({"dry_run": false})));
            r.headers_mut().remove("origin");
            assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN, "{uri}");
            let mut r = req("POST", uri, None);
            r.headers_mut().remove("x-eye-token");
            assert_eq!(call(&e, r).await.0, StatusCode::UNAUTHORIZED, "{uri}");
            // no body or a broken body means dry run
            let (st, v) = call(&e, req("POST", uri, None)).await;
            assert_eq!((st, &v["dry_run"]), (StatusCode::OK, &json!(true)), "{uri}");
        }
        let big = json!({"title": "x".repeat(100_000)});
        assert_eq!(
            call(&e, req("POST", "/api/ideas", Some(big))).await.0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn only_loopback_host_names_with_the_boards_port_are_served() {
        let e = env();
        let with_host = |host: Option<&str>| {
            let mut r = req("GET", "/api/board", None);
            r.headers_mut().remove("host");
            if let Some(h) = host {
                r.headers_mut().insert("host", h.parse().unwrap());
            }
            r
        };
        for ok in [
            "127.0.0.1:7420",
            "localhost:7420",
            "[::1]:7420",
            "LocalHost:7420",
        ] {
            assert_eq!(
                call(&e, with_host(Some(ok))).await.0,
                StatusCode::OK,
                "{ok}"
            );
        }
        // DNS rebinding: a page on another name that resolves to 127.0.0.1 keeps its own Host,
        // and the token alone does not help
        for bad in [
            "evil.example:7420",
            "evil.example",
            "127.0.0.1",
            "127.0.0.1:9999",
            "localhost",
            "[::1]",
            "127.0.0.1:7420.evil.example",
            "127.0.0.1:7420@evil.example",
            "localhost.evil.example:7420",
            "0.0.0.0:7420",
            "",
        ] {
            assert_eq!(
                call(&e, with_host(Some(bad))).await.0,
                StatusCode::FORBIDDEN,
                "{bad:?}"
            );
        }
        assert_eq!(call(&e, with_host(None)).await.0, StatusCode::FORBIDDEN);
        // the check comes first: not even the icons or the page answer a foreign name
        for uri in ["/", "/favicon.svg", "/manifest.webmanifest", "/api/events"] {
            let mut r = req("GET", uri, None);
            r.headers_mut()
                .insert("host", "evil.example:7420".parse().unwrap());
            assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN, "{uri}");
        }
        // an Origin on a write has to be loopback too, in any case
        let mut r = req("POST", "/api/ideas", Some(json!({"title": "x"})));
        r.headers_mut()
            .insert("origin", "http://[::1]:7420".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::OK);
        let mut r = req("POST", "/api/ideas", Some(json!({"title": "x"})));
        r.headers_mut()
            .insert("origin", "http://evil.example:7420".parse().unwrap());
        assert_eq!(call(&e, r).await.0, StatusCode::FORBIDDEN);
    }
}
