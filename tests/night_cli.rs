//! `eye night` through the real binary, with fake `claude` and `codex` scripts first on PATH. No
//! real agent runs; nothing outside the temporary home is touched.

use agent_eye::db::Db;
use agent_eye::paths::Paths;
use std::path::{Path, PathBuf};
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
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// The fake Claude: records its arguments, edits a file, writes the transcript where the adapter
/// reads it, and prints. `FAKE_MODE=ratelimit` prints a rate-limit message and fails.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
echo "$@" > "$HOME/claude-argv.txt"
if [ "$FAKE_MODE" = ratelimit ]; then echo "Error: rate limit reached for your plan" >&2; exit 1; fi
sid=""
while [ $# -gt 0 ]; do
  if [ "$1" = "--session-id" ]; then sid="$2"; fi
  shift
done
cwd=$(pwd -P)
slug=$(printf %s "$cwd" | sed 's/[^A-Za-z0-9]/-/g')
dir="$HOME/.claude/projects/$slug"
mkdir -p "$dir"
echo "hello" >> README.md
echo "extra" > extra.txt
printf '{"type":"user","timestamp":"2026-11-21T01:00:00.000Z","cwd":"%s","sessionId":"%s","message":{"content":"Add a README line saying hello"}}\n' "$cwd" "$sid" > "$dir/$sid.jsonl"
printf '{"type":"assistant","timestamp":"2026-11-21T01:00:05.000Z","cwd":"%s","sessionId":"%s","message":{"content":[{"type":"text","text":"Added the hello line. token ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}]}}\n' "$cwd" "$sid" >> "$dir/$sid.jsonl"
echo "all done"
"#;

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let h = dir.path();
        std::fs::create_dir_all(h.join(".claude/projects")).unwrap();
        std::fs::create_dir_all(h.join("bin")).unwrap();
        let p = h.join("bin/claude");
        std::fs::write(&p, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        // the fake agent needs no sandbox, and a Linux box may have none
        std::fs::create_dir_all(h.join(".agent-eye")).unwrap();
        std::fs::write(
            h.join(".agent-eye/config.toml"),
            "[night]\nallow_unsandboxed = true\n",
        )
        .unwrap();
        Home { dir }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn eye_home(&self) -> PathBuf {
        self.path().join(".agent-eye")
    }
    fn repo(&self) -> PathBuf {
        let r = self.path().join("code/proj");
        std::fs::create_dir_all(&r).unwrap();
        git(&r, &["init", "-q", "-b", "main"]);
        std::fs::write(r.join("README.md"), "# proj\n").unwrap();
        git(&r, &["add", "."]);
        git(&r, &["commit", "-qm", "first"]);
        r
    }
    fn eye(&self, args: &[&str], mode: &str) -> Output {
        let path = format!(
            "{}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin",
            self.path().join("bin").display()
        );
        Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", self.path())
            .env_remove("EYE_HOME")
            .env_remove("EYE_CLAUDE_DIR")
            .env_remove("EYE_CLAUDE_BIN")
            .env("PATH", path)
            .env("FAKE_MODE", mode)
            .env("NO_COLOR", "1")
            .args(args)
            .current_dir(self.path())
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let o = self.eye(args, "");
        let t = text(&o);
        assert!(o.status.success(), "eye {args:?}: {t}");
        t
    }
    fn db(&self) -> Db {
        Db::open(
            &Paths {
                home: self.eye_home(),
            }
            .db_path(),
        )
        .unwrap()
    }
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn runs(h: &Home) -> Vec<serde_json::Value> {
    serde_json::from_str(&h.ok(&["night", "ls", "--json"])).unwrap()
}

#[test]
fn a_planned_idea_runs_in_a_worktree_and_comes_back_as_a_needs_you_card() {
    if !have_git() {
        return;
    }
    let h = Home::new();
    let repo = h.repo();
    h.ok(&[
        "idea",
        "add",
        "readme",
        "--prompt",
        "Add a README line saying hello",
        "--dir",
        repo.to_str().unwrap(),
    ]);
    let out = h.ok(&["night", "add", "idea:1", "--limit", "90m"]);
    assert!(out.contains("planned run 1"), "{out}");
    assert!(out.contains("night shift is off"), "{out}");
    // nothing starts without --yes
    let o = h.eye(&["night", "run-now", "1"], "");
    assert!(!o.status.success() && text(&o).contains("--yes"));
    let out = h.ok(&["night", "run-now", "1", "--yes", "--wait"]);
    assert!(out.contains("done"), "{out}");
    let r = &runs(&h)[0];
    assert_eq!(r["status"], "done", "{r}");
    assert_eq!(r["limit"]["kind"], "minutes");
    let wt = PathBuf::from(r["worktree"].as_str().unwrap());
    assert!(wt.starts_with(h.eye_home().join("night")));
    assert!(r["branch"].as_str().unwrap().starts_with("night/2"));
    // the original repo is as it was: same branch, README unchanged, no extra file
    assert_eq!(git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "# proj\n"
    );
    assert!(!repo.join("extra.txt").exists());
    // the worktree has the edits
    assert!(std::fs::read_to_string(wt.join("README.md"))
        .unwrap()
        .contains("hello"));
    // report masked, diff stat, argv safe
    let report = r["report"].as_str().unwrap();
    assert!(
        report.contains("Added the hello line") && !report.contains("ghp_AAAA"),
        "{report}"
    );
    let diff = r["diff_stat"].as_str().unwrap();
    assert!(
        diff.contains("README.md") && diff.contains("extra.txt"),
        "{diff}"
    );
    let argv = std::fs::read_to_string(h.path().join("claude-argv.txt")).unwrap();
    for w in [
        "-p Add a README line saying hello",
        "--session-id",
        "--disallowedTools",
        "git push",
        "Bash(gh*)",
        "Bash(ssh*)",
        "sandbox",
        "strictAllowlist",
    ] {
        assert!(argv.contains(w), "{w} in {argv}");
    }
    assert!(!argv.contains("force"));
    // the session is indexed and waits for Max
    let sid = r["run_session_id"].as_str().unwrap();
    let s = h.db().session_by_id(sid).unwrap().expect("indexed");
    assert_eq!(s.column_manual.as_deref(), Some("needs_you"));
    assert_eq!(s.next_step.as_deref(), Some("review the night run"));
    assert_eq!(
        s.cwd
            .as_deref()
            .map(|c| Path::new(c).canonicalize().unwrap()),
        Some(wt.canonicalize().unwrap())
    );
    // the log is kept
    assert!(h.ok(&["night", "log", "1"]).contains("all done"));
    // discard refuses uncommitted work without --lose-changes, then removes it
    let o = h.eye(&["night", "discard", "1", "--yes"], "");
    assert!(
        !o.status.success() && text(&o).contains("uncommitted"),
        "{}",
        text(&o)
    );
    h.ok(&["night", "discard", "1", "--yes", "--lose-changes"]);
    assert!(!wt.exists());
    assert!(git(&repo, &["branch", "--list", "night/*"]).is_empty());
}

#[test]
fn switch_on_and_off_and_the_window_and_parallel_land_in_config_toml() {
    let h = Home::new();
    h.ok(&["night", "window", "23:00-05:00"]);
    h.ok(&["night", "parallel", "3"]);
    let o = h.eye(&["night", "window", "07:00"], "");
    assert!(!o.status.success());
    let o = h.eye(&["night", "parallel", "0"], "");
    assert!(!o.status.success());
    h.ok(&["night", "on"]);
    let cfg = std::fs::read_to_string(h.eye_home().join("config.toml")).unwrap();
    assert!(
        cfg.contains("window = \"23:00-05:00\"") && cfg.contains("parallel = 3"),
        "{cfg}"
    );
    assert!(cfg.contains("enabled = true"), "{cfg}");
    assert!(h
        .ok(&["night"])
        .contains("night shift on: window 23:00-05:00"));
    h.ok(&["night", "off"]);
    assert!(h.ok(&["night", "ls"]).contains("night shift off"));
}

#[test]
fn a_weekly_limit_without_data_is_refused_when_planned() {
    if !have_git() {
        return;
    }
    let h = Home::new();
    let repo = h.repo();
    h.ok(&[
        "idea",
        "add",
        "x",
        "--prompt",
        "do",
        "--dir",
        repo.to_str().unwrap(),
    ]);
    let o = h.eye(&["night", "add", "idea:1", "--limit", "80%"], "");
    assert!(!o.status.success());
    assert!(text(&o).contains("no weekly plan data"), "{}", text(&o));
    assert!(runs(&h).is_empty());
    // an idea without a prompt says so
    h.ok(&["idea", "add", "y", "--dir", repo.to_str().unwrap()]);
    let o = h.eye(&["night", "add", "idea:2"], "");
    assert!(text(&o).contains("no prompt"), "{}", text(&o));
    // cancel and ls
    h.ok(&["night", "add", "idea:1"]);
    h.ok(&["night", "cancel", "1"]);
    assert_eq!(runs(&h)[0]["status"], "skipped");
}

#[test]
fn a_rate_limited_run_starts_again_on_the_other_agent_when_it_allows_it() {
    if !have_git() {
        return;
    }
    let h = Home::new();
    std::fs::create_dir_all(h.eye_home()).unwrap();
    std::fs::write(
        h.eye_home().join("config.toml"),
        "[agents]\nenabled = [\"claude\", \"codex\"]\n[night]\nallow_unsandboxed = true\n",
    )
    .unwrap();
    let repo = h.repo();
    h.ok(&[
        "idea",
        "add",
        "x",
        "--prompt",
        "do it",
        "--dir",
        repo.to_str().unwrap(),
        "--agent",
        "claude",
    ]);
    h.ok(&["night", "add", "idea:1", "--switch"]);
    let o = h.eye(&["night", "run-now", "1", "--yes", "--wait"], "ratelimit");
    assert!(o.status.success(), "{}", text(&o));
    let rs = runs(&h);
    let first = rs.iter().find(|r| r["id"] == 1).unwrap();
    assert_eq!(first["status"], "failed");
    assert!(
        first["why"]
            .as_str()
            .unwrap()
            .contains("started again on codex"),
        "{first}"
    );
    let second = rs.iter().find(|r| r["id"] == 2).unwrap();
    assert_eq!(
        (second["agent"].as_str(), second["status"].as_str()),
        (Some("codex"), Some("planned"))
    );
}
