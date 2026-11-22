//! `eye loose`: what a session left behind in a real (temporary) git repo, through the real
//! binary. Nothing here touches a process or a repo of the person running the tests.

use agent_eye::db::Db;
use agent_eye::paths::Paths;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn have_git() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn paths(&self) -> Paths {
        Paths {
            home: self.dir.path().join("eye"),
        }
    }

    fn eye(&self, args: &[&str]) -> Output {
        let path = std::env::var("PATH").unwrap_or_default();
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", self.dir.path())
            .env("EYE_HOME", self.paths().home)
            .env_remove("EYE_CLAUDE_DIR")
            .env("PATH", path)
            .env("NO_COLOR", "1")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn err(&self, args: &[&str]) -> String {
        let o = self.eye(args);
        assert!(!o.status.success(), "{args:?} should fail");
        String::from_utf8_lossy(&o.stderr).into_owned()
    }
}

/// A session in a repo with one commit on its remote, one that is not, and one modified file.
fn world() -> (Home, std::path::PathBuf, std::path::PathBuf) {
    let h = Home {
        dir: tempfile::tempdir().unwrap(),
    };
    h.paths().ensure_home().unwrap();
    let repo = h.dir.path().join("work/repo");
    let remote = h.dir.path().join("remote.git");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "--bare", "-q"]);
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    git(
        &repo,
        &["remote", "add", "origin", &remote.display().to_string()],
    );
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    std::fs::write(repo.join("a.txt"), "two").unwrap();
    git(&repo, &["commit", "-q", "-am", "second"]);
    std::fs::write(repo.join("a.txt"), "three").unwrap();
    std::fs::write(repo.join("untracked.txt"), "x").unwrap();
    let db = Db::open(&h.paths().db_path()).unwrap();
    db.conn
        .execute(
            "INSERT INTO session(id,title,cwd,repo_root,kind,prompt_count,last_activity_at,transcript_state)
             VALUES('abcdef12-0000-4000-8000-000000000001','Work',?1,?1,'interactive',4,?2,'present')",
            rusqlite::params![repo.display().to_string(), agent_eye::util::now_ms()],
        )
        .unwrap();
    (h, repo, remote)
}

#[test]
fn it_lists_commits_on_no_remote_and_modified_files_and_never_pushes() {
    if !have_git() {
        return;
    }
    let (h, repo, remote) = world();
    let out = h.ok(&["loose"]);
    assert!(out.contains("abcdef"), "{out}");
    assert!(out.contains("1 commit in repo is on no remote"), "{out}");
    assert!(
        out.contains("1 modified file in repo is not committed"),
        "{out}"
    );
    assert!(
        out.contains("Leftovers: 1 commit not pushed, 1 file not committed"),
        "{out}"
    );
    let json: serde_json::Value = serde_json::from_str(&h.ok(&["loose", "--json"])).unwrap();
    let kinds: Vec<&str> = json
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["uncommitted", "unpushed"]);
    assert!(json[1]["command"]
        .as_str()
        .unwrap()
        .ends_with("git push --all"));
    // a session prefix limits the list
    assert!(h.ok(&["loose", "abcdef"]).contains("not committed"));
    // it only read: the remote still has one commit and the working copy is as it was
    assert_eq!(git(&remote, &["rev-list", "--count", "main"]).trim(), "1");
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).unwrap(),
        "three"
    );
    // eye does not act on commits or files
    let e = h.err(&["loose", "stop", json[1]["key"].as_str().unwrap(), "--yes"]);
    assert!(
        e.contains("does not act on a unpushed") || e.contains("cannot"),
        "{e}"
    );
}

#[test]
fn an_ignored_item_is_not_listed_again_after_a_new_scan() {
    if !have_git() {
        return;
    }
    let (h, _repo, _remote) = world();
    let json: serde_json::Value = serde_json::from_str(&h.ok(&["loose", "--json"])).unwrap();
    for i in json.as_array().unwrap() {
        h.ok(&["loose", "ignore", i["key"].as_str().unwrap()]);
    }
    assert!(h.ok(&["loose"]).contains("no leftovers"));
    assert_eq!(h.ok(&["loose", "--json"]).trim(), "[]");
    // a key no scan found is refused
    assert!(h
        .err(&["loose", "ignore", "proc:1:1"])
        .contains("no such leftover"));
    assert!(h
        .err(&["loose", "remove", "worktree:/nope", "--yes"])
        .contains("no such leftover"));
}

#[test]
fn with_loose_turned_off_nothing_is_scanned() {
    let (h, _repo, _remote) = world();
    std::fs::write(h.paths().config_path(), "[loose]\nenabled = false\n").unwrap();
    assert!(h.ok(&["loose"]).contains("leftovers are off"));
}
