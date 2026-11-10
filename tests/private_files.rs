//! What eye writes is readable by its owner only: the home folder, the database, the config, the
//! board token, the hook queue, the logs and the archive. A home made earlier with wider rights is
//! tightened.

#[path = "support/fixtures.rs"]
mod fixtures;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn everything_eye_writes_is_private() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = fixtures::claude_home(tmp.path());
    let home = tmp.path().join("eye-home");
    // a home that exists already, open to everyone
    std::fs::create_dir_all(&home).unwrap();
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_agent-eye"));
        c.args(args)
            .env("HOME", tmp.path())
            .env("EYE_HOME", &home)
            .env("EYE_CLAUDE_DIR", &claude)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = c.spawn().unwrap();
        if args[0] == "hook" {
            std::io::Write::write_all(
                child.stdin.as_mut().unwrap(),
                br#"{"session_id":"s1","hook_event_name":"Stop"}"#,
            )
            .unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["scan"]);
    run(&["archive", "--all"]);
    run(&["hook", "Stop"]);
    run(&["board", "on", "--no-service"]);
    run(&["board"]);

    assert_eq!(mode(&home), 0o700, "the home folder");
    for file in ["eye.db", "config.toml", "board.token", "queue/events.jsonl"] {
        assert_eq!(mode(&home.join(file)), 0o600, "{file}");
    }
    for extra in ["eye.db-wal", "eye.db-shm"] {
        if home.join(extra).exists() {
            assert_eq!(mode(&home.join(extra)), 0o600, "{extra}");
        }
    }
    let mut archived = 0;
    for project in std::fs::read_dir(home.join("archive")).unwrap().flatten() {
        for f in std::fs::read_dir(project.path()).unwrap().flatten() {
            assert_eq!(mode(&f.path()), 0o600, "{}", f.path().display());
            archived += 1;
        }
    }
    assert!(archived >= 3, "the fixture sessions were archived");

    // an old database and config that were readable by others are tightened on the next run
    std::fs::set_permissions(home.join("eye.db"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::set_permissions(
        home.join("config.toml"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    std::fs::set_permissions(
        home.join("board.token"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    run(&["scan"]);
    run(&["board"]);
    for file in ["eye.db", "config.toml", "board.token"] {
        assert_eq!(mode(&home.join(file)), 0o600, "{file} after tightening");
    }
}
