//! A machine where nothing is there yet: no Claude Code folder at all and no eye home. Every
//! command must give a clean, sensible result: no panic, a clear message, nothing created that the
//! command was not asked to create.

use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::server::{new_state, router};
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::Value;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use tower::ServiceExt;

/// A HOME with nothing in it. eye resolves `~/.claude` and `~/.agent-eye` from it.
struct Bare {
    home: tempfile::TempDir,
}

impl Bare {
    fn new() -> Bare {
        Bare {
            home: tempfile::tempdir().unwrap(),
        }
    }

    fn eye(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_agent-eye"));
        c.env("HOME", self.home.path())
            .env_remove("EYE_HOME")
            .env_remove("EYE_CLAUDE_DIR")
            .env_remove("EYE_CLAUDE_BIN")
            // no claude on PATH either
            .env("PATH", "/usr/bin:/bin")
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.eye(args).output().unwrap()
    }

    fn entries(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.home.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn scan_finds_nothing_and_says_so() {
    let b = Bare::new();
    let o = b.run(&["scan"]);
    assert!(o.status.success(), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("scanned 0 transcripts"), "{t}");
    // the eye home and its database are made, Claude Code's folder is not
    assert_eq!(b.entries(), [".agent-eye"]);
    let j = b.run(&["scan", "--json"]);
    let v: Value = serde_json::from_slice(&j.stdout).unwrap();
    assert_eq!(v["stats"]["files_seen"], 0);
}

#[test]
fn listing_and_searching_an_empty_index_is_clean() {
    let b = Bare::new();
    for (args, want) in [
        (&["ls"][..], "no sessions"),
        (&["find", "anything"][..], "no sessions match"),
        (&["pick", "--filter", "x"][..], "no sessions match"),
    ] {
        let o = b.run(args);
        assert!(o.status.success(), "{args:?}: {}", text(&o));
        assert!(text(&o).contains(want), "{args:?}: {}", text(&o));
    }
    let j = b.run(&["ls", "--json"]);
    assert_eq!(String::from_utf8_lossy(&j.stdout).trim(), "[]");
    // a session id that does not exist is an error with a reason, not a crash
    let o = b.run(&["show", "abcdef123456"]);
    assert!(!o.status.success());
    assert!(text(&o).contains("no session matches"), "{}", text(&o));
}

#[test]
fn the_picker_without_a_terminal_prints_the_list() {
    let b = Bare::new();
    let mut c = b.eye(&["pick"]);
    // no controlling terminal: the picker must fall back to the plain list instead of a screen
    // SAFETY: setsid between fork and exec only detaches from the controlling terminal.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let o = c.output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("no sessions match"), "{}", text(&o));
    // the bare `eye` does the same
    let mut c = b.eye(&[]);
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    assert!(c.output().unwrap().status.success());
}

#[test]
fn doctor_reports_what_is_missing_without_failing_on_it() {
    let b = Bare::new();
    let o = b.run(&["doctor"]);
    let t = text(&o);
    assert_eq!(o.status.code(), Some(1), "warnings give exit 1: {t}");
    assert!(t.contains("WARN"), "{t}");
    assert!(t.contains("Claude Code not found"), "{t}");
    assert!(t.contains("daemon not running"), "{t}");
    assert!(t.contains("no hook event received yet"), "{t}");
    assert!(!t.contains("panicked"), "{t}");
    // doctor only looks: nothing is created
    assert!(b.entries().is_empty(), "{:?}", b.entries());
}

#[test]
fn a_dry_run_install_writes_nothing() {
    let b = Bare::new();
    let o = b.run(&["install", "--dry-run", "--no-service"]);
    let t = text(&o);
    assert!(o.status.success(), "{t}");
    assert!(t.contains("dry run: nothing was changed"), "{t}");
    assert!(t.contains("none found; following Claude Code"), "{t}");
    assert!(t.contains("would add"), "{t}");
    assert!(t.contains("daemon  would add "), "{t}");
    assert!(b.entries().is_empty(), "{:?}", b.entries());
    // the flag's old name still works
    let old = b.run(&["install", "--dry-run", "--no-launchctl"]);
    assert_eq!(text(&old), t);
}

#[test]
fn a_hook_with_nothing_set_up_never_fails() {
    let b = Bare::new();
    let mut c = b.eye(&["hook", "Stop"]);
    c.stdin(Stdio::piped());
    let mut child = c.spawn().unwrap();
    std::io::Write::write_all(
        child.stdin.as_mut().unwrap(),
        br#"{"session_id":"s1","hook_event_name":"Stop"}"#,
    )
    .unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    assert!(o.stdout.is_empty(), "a hook prints nothing");
}

#[test]
fn marking_the_current_session_without_one_explains_itself() {
    let b = Bare::new();
    let o = b.run(&["mark", "--current", "done"]);
    assert!(!o.status.success());
    assert!(
        text(&o).contains("no session found for this folder"),
        "{}",
        text(&o)
    );
}

#[tokio::test]
async fn the_board_api_of_an_empty_home_is_an_empty_board() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: tmp.path().join("home"),
    };
    paths.ensure_home().unwrap();
    let _ = Db::open(&paths.db_path()).unwrap();
    // Claude Code is enabled and its folder does not exist
    let cfg = Config::with_claude_dir(&tmp.path().join("no-such-claude"));
    let app = router(new_state(paths, cfg, "t".repeat(48), 7420));
    let get = |uri: &'static str| {
        let app = app.clone();
        async move {
            let req = Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:7420")
                .header("x-eye-token", "t".repeat(48))
                .body(Body::empty())
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            assert!(resp.status().is_success(), "{uri}");
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };
    let board = get("/api/board").await;
    let columns = board["columns"].as_array().unwrap();
    assert_eq!(columns.len(), 6);
    assert!(columns
        .iter()
        .all(|c| c["cards"].as_array().unwrap().is_empty()));
    assert_eq!(board["archive_count"], 0);
    assert!(get("/api/archive").await["cards"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(get("/api/ideas").await["ideas"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(get("/api/search?q=anything").await["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(get("/api/agents").await["agents"][0]["id"], "claude");
}
