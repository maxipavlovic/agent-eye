//! Who may talk to the server: the Host and Origin checks, the token (a request header, never a
//! cookie) and the token file.

use super::Shared;
use crate::db::Db;
use crate::paths::Paths;
use crate::util::{hex, random_bytes};
use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{header, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rusqlite::{Transaction, TransactionBehavior};

/// The `meta` key that says the token was replaced once. Up to 0.5.0-rc.3 the token also went into a
/// cookie, which a browser sends to every other server on 127.0.0.1 whatever its port, so a token
/// made before that may have left the machine.
const ROTATED: &str = "board_token_rotated";

/// The one route a browser fetches itself, so it cannot send the token header: the token may come
/// in the address (`?t=`) here and nowhere else.
pub(super) const MANIFEST: &str = "/manifest.webmanifest";

fn read_token(p: &std::path::Path) -> Option<String> {
    let t = std::fs::read_to_string(p).ok()?;
    let t = t.trim();
    (t.len() >= 32).then(|| t.to_string())
}

/// Read `$EYE_HOME/board.token`, creating it (0600, 32 random bytes as hex) when missing. The first
/// time an install that already has a token gets here, the token is replaced (and `eye board`
/// prints the new link); a flag in the database makes that happen once, whoever asks first.
pub fn ensure_token(paths: &Paths) -> Result<String> {
    let p = paths.home.join("board.token");
    let db = Db::open(&paths.db_path())?;
    // One writer at a time, so the daemon and `eye board` cannot both replace it
    let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    let first = db.meta_get(ROTATED)?.is_none();
    let token = match read_token(&p) {
        Some(t) if !first => t,
        _ => {
            let t = hex(&random_bytes(32)?);
            crate::files::write_private(&p, &t)
                .with_context(|| format!("writing {}", p.display()))?;
            t
        }
    };
    if first {
        db.meta_set(ROTATED, "1")?;
    }
    tx.commit()?;
    crate::files::make_private(&p);
    Ok(token)
}

// ---------- guard: Host, Origin, token ----------

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

/// Every request: only the loopback names with the board's own port, and for a write an Origin
/// that is one of them. The page and its files pass on this alone (they hold nothing private);
/// the routes behind `token` need the secret as well.
pub(super) async fn host_and_origin(
    State(st): State<Shared>,
    req: Request,
    next: Next,
) -> Response {
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
    next.run(req).await
}

/// The API and the manifest: the token in `X-Eye-Token` (the manifest may take it as `?t=`). A
/// cookie is not read, so a cookie an older eye set opens nothing.
pub(super) async fn token(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let given = req
        .headers()
        .get("x-eye-token")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .or_else(|| {
            (req.method() == Method::GET && req.uri().path() == MANIFEST)
                .then(|| query_param(req.uri().query(), "t"))
                .flatten()
        });
    if !given.is_some_and(|t| same(&t, &st.token)) {
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

    fn bare(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("host", "127.0.0.1:7420")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn the_page_and_its_files_need_no_token_and_the_api_does() {
        let d = face_dir();
        let e = env_face(Some(d.path().to_path_buf()));
        // The page keeps the token from `?t=` and sends it itself, so it has to load without one.
        for uri in ["/", "/?look=iris", "/assets/app-abc123.js"] {
            let (st, _, body) = raw(&e, bare(uri)).await;
            assert_eq!(st, StatusCode::OK, "{uri}");
            assert!(!body.contains(TOKEN), "{uri}");
        }
        assert_eq!(raw(&e, bare("/nothing")).await.0, StatusCode::NOT_FOUND);
        // Everything that holds sessions, and the manifest (its start URL has the token), does need it
        for uri in [
            "/api/board",
            "/api/events",
            "/api/sessions/x/export?format=html",
            "/manifest.webmanifest",
        ] {
            assert_eq!(
                raw(&e, bare(uri)).await.0,
                StatusCode::UNAUTHORIZED,
                "{uri}"
            );
            let wrong = Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:7420")
                .header("x-eye-token", "nope")
                .body(Body::empty())
                .unwrap();
            assert_eq!(raw(&e, wrong).await.0, StatusCode::UNAUTHORIZED, "{uri}");
        }
        // the Host check comes before everything, the page included
        for uri in ["/", "/assets/app-abc123.js", "/nothing", "/api/board"] {
            let r = Request::builder()
                .uri(uri)
                .header("host", "evil.example:7420")
                .header("x-eye-token", TOKEN)
                .body(Body::empty())
                .unwrap();
            assert_eq!(raw(&e, r).await.0, StatusCode::FORBIDDEN, "{uri}");
        }
    }

    #[tokio::test]
    async fn the_token_is_never_put_in_a_cookie() {
        // A cookie goes to every server on 127.0.0.1 whatever its port; nothing eye sends sets one
        // that holds the token, and the landing serves the page instead of redirecting.
        let d = face_dir();
        let e = env_face(Some(d.path().to_path_buf()));
        let landing = format!("/?t={TOKEN}&look=ink");
        let (st, h, body) = raw(&e, bare(&landing)).await;
        assert_eq!(st, StatusCode::OK);
        assert!(body.contains("<title>face</title>") && !body.contains(TOKEN));
        for uri in [
            landing.as_str(),
            "/",
            "/assets/app-abc123.js",
            "/api/board",
            "/api/search?q=x",
            "/manifest.webmanifest",
        ] {
            let (_, h, _) = raw(&e, req("GET", uri, None)).await;
            for v in h.get_all(header::SET_COOKIE) {
                assert!(!v.to_str().unwrap().contains(TOKEN), "{uri}");
            }
        }
        // the only Set-Cookie the landing sends takes the old cookie away
        let set: Vec<_> = h.get_all(header::SET_COOKIE).iter().collect();
        assert_eq!(set.len(), 1);
        let c = set[0].to_str().unwrap();
        assert!(c.starts_with("eye_token=;") && c.contains("Max-Age=0") && c.contains("Path=/"));
    }

    #[tokio::test]
    async fn a_cookie_from_an_older_eye_opens_nothing_and_is_cleared_on_the_next_visit() {
        let d = face_dir();
        let e = env_face(Some(d.path().to_path_buf()));
        let old = Request::builder()
            .uri("/api/board")
            .header("host", "127.0.0.1:7420")
            .header("cookie", format!("a=b; eye_token={TOKEN}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(raw(&e, old).await.0, StatusCode::UNAUTHORIZED);
        // a visit to the page, with or without the link, takes the cookie away
        let visit = Request::builder()
            .uri("/")
            .header("host", "127.0.0.1:7420")
            .header("cookie", format!("a=b; eye_token={TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let (st, h, _) = raw(&e, visit).await;
        assert_eq!(st, StatusCode::OK);
        let c = h[header::SET_COOKIE].to_str().unwrap();
        assert!(
            c.starts_with("eye_token=;") && c.contains("Max-Age=0"),
            "{c}"
        );
        // and a visit with no old cookie sets nothing
        let (_, h, _) = raw(&e, bare("/")).await;
        assert!(h.get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn the_address_carries_the_token_for_the_manifest_only() {
        let d = face_dir();
        let e = env_face(Some(d.path().to_path_buf()));
        let (st, _, body) = raw(&e, bare(&format!("/manifest.webmanifest?t={TOKEN}"))).await;
        assert_eq!(st, StatusCode::OK);
        assert!(body.contains(&format!("/?t={TOKEN}")));
        for uri in [
            format!("/api/board?t={TOKEN}"),
            format!("/api/events?t={TOKEN}"),
            "/manifest.webmanifest?t=nope".to_string(),
        ] {
            assert_eq!(
                raw(&e, bare(&uri)).await.0,
                StatusCode::UNAUTHORIZED,
                "{uri}"
            );
        }
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

        let with_token = Request::builder()
            .uri("/api/board")
            .header("host", "localhost:7420")
            .header("x-eye-token", TOKEN)
            .body(Body::empty())
            .unwrap();
        assert_eq!(call(&e, with_token).await.0, StatusCode::OK);

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

    fn home() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().join("h"),
        };
        paths.ensure_home().unwrap();
        (tmp, paths)
    }

    #[test]
    fn token_file_is_private_and_stable() {
        let (_tmp, paths) = home();
        let t1 = ensure_token(&paths).unwrap();
        assert_eq!(t1.len(), 64);
        assert_eq!(ensure_token(&paths).unwrap(), t1);
        let mode = std::fs::metadata(paths.home.join("board.token")).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
            0o600
        );
    }

    #[test]
    fn a_token_made_before_the_cookie_was_dropped_is_replaced_once() {
        let (_tmp, paths) = home();
        let old = "a".repeat(64);
        std::fs::write(paths.home.join("board.token"), &old).unwrap();
        let new = ensure_token(&paths).unwrap();
        assert_ne!(new, old);
        assert_eq!(new.len(), 64);
        assert_eq!(
            std::fs::read_to_string(paths.home.join("board.token")).unwrap(),
            new
        );
        // once: every later look (the daemon, `eye board`) gets the same one
        for _ in 0..3 {
            assert_eq!(ensure_token(&paths).unwrap(), new);
        }
        let mode = std::fs::metadata(paths.home.join("board.token")).unwrap();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode.permissions()) & 0o777,
            0o600
        );
    }

    #[test]
    fn a_new_install_gets_its_token_without_a_second_one() {
        let (_tmp, paths) = home();
        let first = ensure_token(&paths).unwrap();
        // the flag is already set: a restart keeps this token
        assert_eq!(ensure_token(&paths).unwrap(), first);
        // a token the person put there by hand after that is theirs
        let mine = "b".repeat(48);
        std::fs::write(paths.home.join("board.token"), &mine).unwrap();
        assert_eq!(ensure_token(&paths).unwrap(), mine);
    }

    #[test]
    fn two_askers_at_once_agree_on_the_replaced_token() {
        let (_tmp, paths) = home();
        std::fs::write(paths.home.join("board.token"), "c".repeat(64)).unwrap();
        let handles: Vec<_> = (0..6)
            .map(|_| {
                let p = Paths {
                    home: paths.home.clone(),
                };
                std::thread::spawn(move || ensure_token(&p).unwrap())
            })
            .collect();
        let tokens: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(tokens.iter().all(|t| *t == tokens[0]), "{tokens:?}");
        assert_eq!(ensure_token(&paths).unwrap(), tokens[0]);
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
