//! The read commands where eye's folder cannot be written, as in a night run's sandbox, through the
//! real binary. A folder and its files made read-only stand in for the sandbox; the real one was
//! tried with `sandbox-exec` (see the CHANGELOG entry). Skipped as root, which writes anywhere.

use agent_eye::db::Db;
use agent_eye::ideas::{self, NewIdea};
use agent_eye::paths::Paths;
use agent_eye::util::now_ms;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const SID: &str = "c1e00001-0000-4000-8000-000000000000";

fn as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

fn set_mode(p: &Path, mode: u32) {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

struct Home {
    dir: tempfile::TempDir,
    locked: bool,
}

impl Home {
    /// A home with one session and one idea, its config written when `config` is set, and its
    /// database closed (so no `-wal` or `-shm` file is left).
    fn new(config: bool) -> Home {
        let h = Home {
            dir: tempfile::tempdir().unwrap(),
            locked: false,
        };
        h.paths().ensure_home().unwrap();
        if config {
            std::fs::write(h.paths().config_path(), "[summary]\nenabled = false\n").unwrap();
        }
        let db = h.db();
        h.session(&db, SID, "Retry failed webhook deliveries");
        ideas::add(
            &db,
            &NewIdea {
                title: "Look at the queue",
                notes: None,
                prompt: None,
                folder: Some("/tmp/demo"),
                from_session: None,
                agent: None,
            },
        )
        .unwrap();
        h
    }

    fn paths(&self) -> Paths {
        Paths {
            home: self.dir.path().join("eye"),
        }
    }

    fn db(&self) -> Db {
        Db::open(&self.paths().db_path()).unwrap()
    }

    fn session(&self, db: &Db, id: &str, title: &str) {
        db.conn
            .execute(
                "INSERT INTO session(id, agent, cwd, title, first_prompt, started_at, last_activity_at, kind,
                   transcript_state, prompt_count, reply_count, ai_status)
                 VALUES(?1,'claude','/tmp/demo',?2,?2,?3,?3,'interactive','gone',2,2,'needs_you')",
                rusqlite::params![id, title, now_ms() - 60_000],
            )
            .unwrap();
        agent_eye::db::fts_write(&db.conn, id, title, "ok", true, 8_000, 8_000).unwrap();
    }

    /// Take the right to write away from the folder and every file in it.
    fn lock(&mut self) {
        let home = self.paths().home;
        for e in std::fs::read_dir(&home).unwrap().flatten() {
            set_mode(&e.path(), 0o400);
        }
        set_mode(&home, 0o500);
        self.locked = true;
    }

    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.paths().home)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn eye(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", self.dir.path())
            .env("EYE_HOME", self.paths().home)
            .env_remove("EYE_CLAUDE_DIR")
            .env("PATH", "/usr/bin:/bin")
            .env("TZ", "UTC")
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
}

impl Drop for Home {
    fn drop(&mut self) {
        if self.locked {
            let home = self.paths().home;
            set_mode(&home, 0o700);
            for e in std::fs::read_dir(&home).into_iter().flatten().flatten() {
                set_mode(&e.path(), 0o600);
            }
        }
    }
}

/// Every command that reads, on a locked home: each answers, and none makes or changes a file.
fn reads_everything(h: &Home) {
    let before = h.files();
    assert!(h.ok(&["find", "webhook"]).contains("Retry failed webhook"));
    let ls = h.ok(&["ls", "--json"]);
    assert!(ls.contains(SID), "{ls}");
    assert!(h.ok(&["show", &SID[..8]]).contains("Retry failed webhook"));
    assert!(h.ok(&["show", &SID[..8], "--json"]).contains(SID));
    assert!(h.ok(&["idea", "ls"]).contains("Look at the queue"));
    assert!(h.ok(&["night", "ls"]).contains("night shift"));
    assert!(h.ok(&["usage"]).contains("no usage in this window"));
    assert!(h.ok(&["loose"]).contains("no leftovers"));
    let said = String::from_utf8_lossy(&h.eye(&["loose"]).stderr).into_owned();
    assert!(said.contains("what the daemon last stored"), "{said}");
    assert!(!h.ok(&["clock", "--brief"]).trim().is_empty());
    assert!(h.ok(&["clock"]).contains("Steering clock"));
    assert!(h.ok(&["remind", "ls"]).contains("no reminders"));
    assert!(h.ok(&["resume", &SID[..8], "--dry-run"]).contains(SID));
    assert_eq!(h.files(), before, "a read command made or changed a file");
}

/// Every command that writes refuses with `said` in its message, and changes nothing.
fn refuses_to_write(h: &Home, said: &str) {
    let before = h.files();
    for args in [
        vec!["hide", &SID[..8]],
        vec!["idea", "add", "New idea"],
        vec!["mark", &SID[..8], "done"],
        vec!["remind", &SID[..8], "tomorrow"],
    ] {
        let err = h.err(&args);
        assert!(
            err.contains(said) && !err.contains("creating"),
            "{args:?}: {err}"
        );
    }
    assert_eq!(h.files(), before);
}

#[test]
fn every_read_command_works_on_a_home_that_cannot_be_written() {
    if as_root() {
        return;
    }
    let mut h = Home::new(true);
    assert_eq!(h.files(), ["config.toml", "eye.db"]);
    h.lock();
    reads_everything(&h);
    refuses_to_write(&h, "cannot be opened for writing");
}

#[test]
fn they_work_without_a_config_file_too() {
    if as_root() {
        return;
    }
    let mut h = Home::new(false);
    assert_eq!(h.files(), ["eye.db"]);
    h.lock();
    reads_everything(&h);
    refuses_to_write(&h, "config.toml");
    assert_eq!(h.files(), ["eye.db"]);
}

#[test]
fn they_read_what_a_running_daemon_has_in_its_log() {
    if as_root() {
        return;
    }
    let mut h = Home::new(true);
    // a process that keeps the database open, with a session that only its log holds
    let writer = h.db();
    writer
        .conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    h.session(
        &writer,
        "c1e00002-0000-4000-8000-000000000000",
        "Only in the log",
    );
    assert!(h.files().contains(&"eye.db-wal".to_string()));
    h.lock();
    assert!(h.ok(&["find", "log"]).contains("Only in the log"));
    reads_everything(&h);
    refuses_to_write(&h, "cannot be opened for writing");
    drop(writer);
}

#[test]
fn a_home_of_another_schema_is_refused_where_it_cannot_be_upgraded() {
    if as_root() {
        return;
    }
    let mut h = Home::new(true);
    h.db().meta_set("schema_version", "17").unwrap();
    h.lock();
    let said = h.err(&["ls"]);
    assert!(
        said.contains("older eye") && said.contains("cannot be upgraded here"),
        "{said}"
    );
}
