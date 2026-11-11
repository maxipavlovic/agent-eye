//! `eye remind`: set, clear and list reminders on sessions and ideas, through the real binary.

use agent_eye::db::Db;
use agent_eye::paths::Paths;
use std::process::{Command, Output, Stdio};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        let h = Home {
            dir: tempfile::tempdir().unwrap(),
        };
        h.paths().ensure_home().unwrap();
        let db = Db::open(&h.paths().db_path()).unwrap();
        db.conn
            .execute(
                "INSERT INTO session(id,title,cwd,kind,prompt_count,last_activity_at,transcript_state)
                 VALUES('abcdef12-0000-4000-8000-000000000001','Waiting on Ana','/tmp/demo','interactive',4,1000,'present')",
                [],
            )
            .unwrap();
        h
    }

    fn paths(&self) -> Paths {
        Paths {
            home: self.dir.path().join("eye"),
        }
    }

    fn eye(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", self.dir.path())
            .env("EYE_HOME", self.paths().home)
            .env_remove("EYE_CLAUDE_DIR")
            .env("PATH", "/usr/bin:/bin")
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    /// stdout of a command that must succeed.
    fn ok(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// stderr of a command that must fail.
    fn err(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        assert!(!o.status.success(), "{args:?} should fail");
        String::from_utf8_lossy(&o.stderr).into_owned()
    }

    fn db(&self) -> Db {
        Db::open(&self.paths().db_path()).unwrap()
    }
}

#[test]
fn a_reminder_is_set_listed_and_cleared_on_a_session_and_an_idea() {
    let h = Home::new();
    assert!(h.ok(&["remind", "ls"]).contains("no reminders"));
    let out = h.ok(&["remind", "abcdef", "tomorrow", "--note", "Ana's reply"]);
    assert!(
        out.contains("will remind you tomorrow 09:00: Waiting on Ana"),
        "{out}"
    );
    h.ok(&["idea", "add", "Look at the queue"]);
    h.ok(&["remind", "idea:1", "2026-12-31", "23:30"]);
    let list = h.ok(&["remind", "ls"]);
    let lines: Vec<&str> = list.lines().collect();
    assert_eq!(lines.len(), 2, "{list}");
    assert!(
        lines[0].contains("abcdef12") && lines[0].contains("(Ana's reply)"),
        "{list}"
    );
    assert!(
        lines[1].contains("idea:1") && lines[1].contains("23:30"),
        "{list}"
    );
    assert!(!list.contains("DUE"));

    // a time that has come is listed first and marked
    h.db()
        .conn
        .execute("UPDATE idea SET remind_at=1", [])
        .unwrap();
    let list = h.ok(&["remind", "ls"]);
    assert!(list.lines().next().unwrap().starts_with("DUE"), "{list}");
    assert!(list.lines().next().unwrap().contains("idea:1"), "{list}");

    assert!(h.ok(&["remind", "abcdef", "--clear"]).contains("cleared"));
    assert!(!h.ok(&["remind", "ls"]).contains("abcdef12"));
    h.ok(&["remind", "idea:1", "--clear"]);
    assert!(h.ok(&["remind", "ls"]).contains("no reminders"));
}

#[test]
fn marking_a_session_done_ends_its_reminder() {
    let h = Home::new();
    h.ok(&["remind", "abcdef", "3d"]);
    h.ok(&["mark", "abcdef", "done"]);
    assert!(h.ok(&["remind", "ls"]).contains("no reminders"));
}

#[test]
fn mistakes_are_explained() {
    let h = Home::new();
    assert!(h.err(&["remind", "abcdef", "soon"]).contains("tomorrow"));
    assert!(h
        .err(&["remind", "abcdef", "2020-01-01"])
        .contains("already passed"));
    assert!(h.err(&["remind", "abcdef"]).contains("say when"));
    assert!(h
        .err(&["remind", "abcdef", "1d", "--clear"])
        .contains("--clear"));
    assert!(h
        .err(&["remind", "zzzzzz", "1d"])
        .contains("no session matches"));
    assert!(h.err(&["remind", "idea:9", "1d"]).contains("no idea 9"));
    assert!(h.ok(&["remind", "ls"]).contains("no reminders"));
}
