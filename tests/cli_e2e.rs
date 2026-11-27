//! The commands people type, end to end through the real binary on temporary homes: `eye usage`,
//! `eye loose` (stop and remove on things this test made), `eye night`, `eye find`/`ls`/`show`/
//! `pick`, `eye idea` and the export/import round trip through `-`. Every home is a temporary
//! folder (`HOME` included), PATH holds a fake `claude` that only fails, and no real agent runs.

use agent_eye::db::Db;
use agent_eye::loose::{Probe, System};
use agent_eye::paths::Paths;
use agent_eye::usage::{day_of, limits, LimitWindow, PlanLimits};
use agent_eye::util::now_ms;
use rusqlite::params;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const DAY: i64 = 86_400_000;
const MIN: i64 = 60_000;

fn have(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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

fn sid(n: u8) -> String {
    format!("c1e{n:05}-0000-4000-8000-000000000000")
}

struct World {
    /// The canonical path: git lists worktrees by it.
    root: PathBuf,
    _tmp: tempfile::TempDir,
}

impl World {
    fn new() -> World {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for d in ["home", "claude/projects", "bin", "work"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let fake = root.join("bin/claude");
        std::fs::write(&fake, "#!/bin/sh\necho not run >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let w = World { root, _tmp: tmp };
        w.paths().ensure_home().unwrap();
        // no summaries (they would call the agent), and the night shift stays off
        std::fs::write(
            w.paths().config_path(),
            "[summary]\nenabled = false\n\n[night]\nenabled = false\n",
        )
        .unwrap();
        w
    }

    fn paths(&self) -> Paths {
        Paths {
            home: self.root.join("eye"),
        }
    }

    fn db(&self) -> Db {
        Db::open(&self.paths().db_path()).unwrap()
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_agent-eye"));
        c.env_clear()
            .env("HOME", self.root.join("home"))
            .env("EYE_HOME", self.paths().home)
            .env("EYE_CLAUDE_DIR", self.root.join("claude"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("TZ", "UTC")
            .env("NO_COLOR", "1");
        // keeps the binary's own coverage when the suite runs under `cargo llvm-cov`
        if let Some(v) = std::env::var_os("LLVM_PROFILE_FILE") {
            c.env("LLVM_PROFILE_FILE", v);
        }
        c
    }

    /// Run with `stdin` piped (nothing is not a terminal either).
    fn run(&self, args: &[&str], stdin: &[u8]) -> Output {
        let mut child = self
            .cmd()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args, b"");
        assert!(
            o.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// A failing command: what it said on stderr.
    fn err(&self, args: &[&str]) -> String {
        let o = self.run(args, b"");
        assert!(
            !o.status.success(),
            "{args:?} should fail, said {}",
            String::from_utf8_lossy(&o.stdout)
        );
        String::from_utf8_lossy(&o.stderr).into_owned()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    /// A folder under `work/`.
    fn dir(&self, name: &str) -> PathBuf {
        let d = self.root.join("work").join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A session `ago_min` minutes old in `cwd`, searchable by its title and prompt.
    fn session(&self, n: u8, title: &str, cwd: &Path, ai: Option<&str>, ago_min: i64) -> String {
        let db = self.db();
        let id = sid(n);
        let last = now_ms() - ago_min * MIN;
        db.conn
            .execute(
                "INSERT INTO session(id, agent, cwd, repo_root, title, title_source, first_prompt, started_at, last_activity_at,
                   kind, transcript_state, prompt_count, reply_count, last_assistant_text, ai_status, state_line, next_step, summary_at)
                 VALUES(?1,'claude',?2,?2,?3,'ai',?4,?5,?6,'interactive','gone',5,5,'ok',?7,'state of it','You: look at it',?6)",
                params![id, cwd.display().to_string(), title, format!("please {title}"), last - 30 * MIN, last, ai],
            )
            .unwrap();
        agent_eye::db::fts_write(
            &db.conn,
            &id,
            &format!("please {title}"),
            "ok",
            true,
            8_000,
            8_000,
        )
        .unwrap();
        id
    }

    /// One day of use of a session.
    fn usage(&self, sid: &str, days_ago: i64, model: &str, t: [i64; 4]) {
        // the eye under test runs with TZ=UTC, so its days are UTC days
        let day = day_of(now_ms() - days_ago * DAY, &|_| 0);
        let db = self.db();
        db.conn
            .execute(
                "INSERT INTO usage_day(session_id, day, model, tok_in, tok_out, tok_cw, tok_cr) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![sid, day, model, t[0], t[1], t[2], t[3]],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT OR IGNORE INTO usage_state(session_id, unreadable) VALUES(?1,0)",
                [sid],
            )
            .unwrap();
    }

    fn loose_row(&self, session: &str, key: &str, kind: &str, line: &str, detail: &str) {
        self.db()
            .conn
            .execute(
                "INSERT OR REPLACE INTO loose(session_id, key, kind, line, detail, since_ms, found_at) VALUES(?1,?2,?3,?4,?5,?6,?6)",
                params![session, key, kind, line, detail, now_ms()],
            )
            .unwrap();
    }
}

// ---------------------------------------------------------------------------------------- usage

/// Two sessions of two agents, two models, three days.
fn usage_world() -> World {
    let w = World::new();
    let a = w.session(1, "Add dark mode", &w.dir("web-shop"), None, 10);
    let b = w.session(2, "Port the engine", &w.dir("billing"), None, 20);
    w.db()
        .conn
        .execute("UPDATE session SET agent='codex' WHERE id=?1", [&b])
        .unwrap();
    w.usage(&a, 0, "claude-opus-5-5", [1_000_000, 100_000, 0, 5_000_000]);
    w.usage(&a, 2, "claude-sonnet-5-5", [2_000_000, 200_000, 0, 0]);
    w.usage(&b, 1, "gpt-6-luna", [3_000_000, 300_000, 0, 0]);
    w.usage(&b, 40, "gpt-6-luna", [9_000_000, 0, 0, 0]);
    w
}

#[test]
fn usage_with_nothing_counted_says_so_and_json_is_empty() {
    let w = World::new();
    let out = w.ok(&["usage"]);
    assert!(out.contains("no usage in this window"), "{out}");
    assert!(out.contains("not your bill"), "{out}");
    let j = w.json(&["usage", "--json"]);
    assert_eq!(j["rows"].as_array().unwrap().len(), 0);
    assert_eq!(j["totals"]["total"], 0);
    assert_eq!(j["days"], 7);
}

#[test]
fn usage_by_day_lists_every_day_of_the_window_and_json_gets_the_series() {
    let w = usage_world();
    let out = w.ok(&["usage", "--by-day", "--days", "3"]);
    assert!(out.contains("day") && out.contains("total"), "{out}");
    let day_rows = out
        .lines()
        .filter(|l| l.starts_with("20") && !l.contains(" to "))
        .count();
    assert_eq!(day_rows, 3, "{out}");
    let j = w.json(&["usage", "--json", "--by-day", "--days", "3"]);
    assert_eq!(j["series"]["days"].as_array().unwrap().len(), 3);
    assert_eq!(w.json(&["usage", "--json"]).get("series"), None);
}

#[test]
fn usage_by_day_follows_by_for_project_and_model_and_refuses_the_rest() {
    let w = usage_world();
    let j = w.json(&[
        "usage", "--json", "--by-day", "--by", "model", "--days", "3",
    ]);
    assert_eq!(j["series"]["by"], "model");
    assert!(w
        .err(&["usage", "--by-day", "--by", "agent"])
        .contains("--by-day splits the days by project or model"));
}

#[test]
fn usage_groups_by_project_agent_model_and_session() {
    let w = usage_world();
    let by = |what: &str| w.ok(&["usage", "--by", what]);
    let project = by("project");
    assert!(
        project.contains("est. cost") && project.contains("total"),
        "{project}"
    );
    // the folders are long temporary paths, so the names are read from the JSON
    let names = |by: &str| -> Vec<String> {
        w.json(&["usage", "--json", "--by", by])["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect()
    };
    // by folder: one row per folder, as it was
    let folders = names("folder");
    assert!(
        folders.iter().any(|n| n.ends_with("web-shop"))
            && folders.iter().any(|n| n.ends_with("billing")),
        "{folders:?}"
    );
    // by project: folders under the temp dir are not projects, so they fold into one row
    assert_eq!(names("project"), vec!["other (2)".to_string()]);
    let agent = by("agent");
    assert!(
        agent.contains("claude") && agent.contains("codex"),
        "{agent}"
    );
    let model = by("model");
    for m in ["claude-opus-5-5", "claude-sonnet-5-5", "gpt-6-luna"] {
        assert!(model.contains(m), "{model}");
    }
    let session = by("session");
    assert!(session.contains("c1e0000"), "{session}");
    assert!(
        session.contains("Add dark mode") && session.contains("Port the engine"),
        "{session}"
    );
}

#[test]
fn usage_windows_count_only_their_days_and_the_40_day_old_use_stays_out_of_a_week() {
    let w = usage_world();
    let tokens = |args: &[&str]| w.json(args)["totals"]["input"].as_i64().unwrap();
    assert_eq!(tokens(&["usage", "--json", "--days", "1"]), 1_000_000);
    assert_eq!(tokens(&["usage", "--json"]), 6_000_000);
    assert_eq!(tokens(&["usage", "--json", "--week"]), 6_000_000);
    assert_eq!(tokens(&["usage", "--json", "--month"]), 6_000_000);
    assert_eq!(tokens(&["usage", "--json", "--days", "60"]), 15_000_000);
    let j = w.json(&["usage", "--json", "--by", "model"]);
    assert_eq!(j["by"], "model");
    assert_eq!(j["rows"].as_array().unwrap().len(), 3);
    assert_eq!(j["top_sessions"].as_array().unwrap().len(), 2);
}

#[test]
fn usage_prints_the_plan_limits_it_has_seen() {
    let w = usage_world();
    limits::record(
        &w.db().conn,
        "claude",
        &PlanLimits {
            seen_at: now_ms() - 5 * MIN,
            plan: Some("Max".into()),
            windows: vec![LimitWindow {
                name: "five_hour".into(),
                used_percent: 62.0,
                window_minutes: Some(300),
                resets_at: Some(now_ms() + 2 * 3_600_000),
            }],
        },
    )
    .unwrap();
    let out = w.ok(&["usage"]);
    assert!(out.contains("62% of the 5-hour limit"), "{out}");
    let j = w.json(&["usage", "--json"]);
    assert_eq!(j["limits"][0]["agent"], "claude");
}

#[test]
fn usage_refuses_a_bad_group_a_bad_day_count_and_clashing_flags() {
    let w = World::new();
    assert!(w
        .err(&["usage", "--by", "colour"])
        .contains("project, folder, agent, model or session"));
    assert!(w.err(&["usage", "--days", "0"]).contains("from 1"));
    assert!(w
        .err(&["usage", "--week", "--month"])
        .contains("cannot be used"));
    assert!(w
        .err(&["usage", "--days", "3", "--month"])
        .contains("cannot be used"));
}

// ---------------------------------------------------------------------------------------- loose

#[test]
fn loose_stop_asks_first_and_ends_a_process_this_test_started() {
    if !have("sleep") {
        return;
    }
    let w = World::new();
    let id = w.session(1, "Run a server", &w.dir("srv"), None, 5);
    let mut child = Command::new("sleep").arg("600").spawn().unwrap();
    let info = System::new().proc_info(child.id()).expect("the child runs");
    let key = format!("proc:{}:{}", child.id(), info.start);
    w.loose_row(
        &id,
        &key,
        "process",
        "sleep is still running",
        &format!("sleep (pid {})", child.id()),
    );

    // without --yes and without a terminal, nothing is stopped
    let said = w.err(&["loose", "stop", &key]);
    assert!(said.contains("not a terminal"), "{said}");
    assert!(
        child.try_wait().unwrap().is_none(),
        "the process must still run"
    );

    let out = w.ok(&["loose", "stop", &key, "--yes"]);
    assert!(out.contains("SIGTERM"), "{out}");
    let status = child.wait().unwrap();
    assert!(!status.success());
    // the item is gone, and a second stop says so instead of killing a stranger
    let again = w.err(&["loose", "stop", &key, "--yes"]);
    assert!(
        again.contains("no such leftover") || again.contains("not running"),
        "{again}"
    );
}

#[test]
fn loose_stop_never_touches_a_pid_that_now_belongs_to_another_process() {
    if !have("sleep") {
        return;
    }
    let w = World::new();
    let id = w.session(1, "Run a server", &w.dir("srv"), None, 5);
    let mut child = Command::new("sleep").arg("600").spawn().unwrap();
    let info = System::new().proc_info(child.id()).unwrap();
    // the start time in the key is not the process's: it is a reused pid
    let key = format!("proc:{}:{}", child.id(), info.start - 1000);
    w.loose_row(&id, &key, "process", "sleep is still running", "sleep");
    let said = w.err(&["loose", "stop", &key, "--yes"]);
    assert!(said.contains("another process"), "{said}");
    assert!(
        child.try_wait().unwrap().is_none(),
        "a stranger's process was stopped"
    );
    child.kill().unwrap();
    child.wait().unwrap();
}

/// A repo with a worktree `wt` on branch `side`; returns (repo, worktree).
fn repo_with_worktree(w: &World) -> (PathBuf, PathBuf) {
    let repo = w.dir("repo");
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    let wt = w.root.join("work/wt");
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    (repo, wt)
}

#[test]
fn loose_remove_takes_a_clean_worktree_away_after_a_yes_and_refuses_a_dirty_one() {
    if !have("git") {
        return;
    }
    let w = World::new();
    let (repo, wt) = repo_with_worktree(&w);
    let id = w.session(1, "Try a side branch", &repo, None, 5);
    let key = format!("worktree:{}", wt.display());
    w.loose_row(
        &id,
        &key,
        "worktree",
        "worktree wt is not in a folder you work in",
        &format!("wt (side)\nrepo {}", repo.display()),
    );

    let said = w.err(&["loose", "remove", &key]);
    assert!(said.contains("not a terminal"), "{said}");
    assert!(wt.exists());

    // an untracked file makes git refuse; eye passes its words on and removes nothing
    std::fs::write(wt.join("scratch.txt"), "x").unwrap();
    let refused = w.err(&["loose", "remove", &key, "--yes"]);
    assert!(
        refused.to_lowercase().contains("untracked") || refused.contains("scratch"),
        "{refused}"
    );
    assert!(wt.exists());

    std::fs::remove_file(wt.join("scratch.txt")).unwrap();
    let out = w.ok(&["loose", "remove", &key, "--yes"]);
    assert!(out.to_lowercase().contains("remov"), "{out}");
    assert!(!wt.exists());
    assert_eq!(git(&repo, &["worktree", "list"]).lines().count(), 1);
}

#[test]
fn loose_remove_and_stop_refuse_the_wrong_kind_and_an_unknown_key() {
    let w = World::new();
    let id = w.session(1, "Work", &w.dir("repo"), None, 5);
    w.loose_row(
        &id,
        "uncommitted:/x",
        "uncommitted",
        "3 files not committed",
        "3 files\nrepo /x",
    );
    assert!(w
        .err(&["loose", "stop", "uncommitted:/x", "--yes"])
        .contains("does not act on"));
    assert!(w
        .err(&["loose", "remove", "worktree:/nowhere", "--yes"])
        .contains("no such leftover"));
}

#[test]
fn loose_ignore_by_key_and_send_dry_run_name_the_session() {
    let w = World::new();
    let cwd = w.dir("repo");
    let id = w.session(1, "Work", &cwd, None, 5);
    w.loose_row(
        &id,
        "uncommitted:/x",
        "uncommitted",
        "3 files not committed",
        "3 files\nrepo /x",
    );
    assert!(w
        .ok(&["loose", "ignore", "uncommitted:/x"])
        .contains("ignored"));
    let j = w.json(&["loose", "--json"]);
    assert!(
        j.as_array()
            .unwrap()
            .iter()
            .all(|i| i["key"] != "uncommitted:/x"),
        "{j}"
    );
    // nothing left for the session now
    assert!(w
        .ok(&["loose", "send", &id[..8], "--dry-run"])
        .contains("no leftovers"));
}

// ----------------------------------------------------------------------------------- find, ls, show, pick

fn finder() -> (World, String, String) {
    let w = World::new();
    let a = w.session(
        1,
        "Retry failed webhook deliveries",
        &w.dir("payments-api"),
        Some("needs_you"),
        30,
    );
    let b = w.session(
        2,
        "Fix stale tokens in the account store",
        &w.dir("auth-service"),
        Some("done"),
        90,
    );
    w.db()
        .conn
        .execute("UPDATE session SET origin='night' WHERE id=?1", [&b])
        .unwrap();
    (w, a, b)
}

#[test]
fn find_ranks_by_words_filters_and_prints_json() {
    let (w, a, _) = finder();
    let out = w.ok(&["find", "webhook"]);
    assert!(out.contains("Retry failed webhook deliveries"), "{out}");
    assert!(!out.contains("stale tokens"), "{out}");
    let none = w.ok(&["find", "zzzzqqqq"]);
    assert!(!none.contains("Retry"), "{none}");
    let j = w.json(&["find", "tokens", "--json"]);
    assert_eq!(j.as_array().unwrap().len(), 1);
    assert!(j[0].to_string().contains("stale tokens"));
    // score keeps 0.2's lower-is-better order; relevance is the same number turned around
    let relevance = j[0]["relevance"].as_f64().unwrap();
    assert!(relevance > 0.0, "{}", j[0]);
    assert_eq!(j[0]["score"].as_f64().unwrap(), -relevance);
    assert_eq!(
        w.json(&["find", "failed", "--json", "--limit", "1"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(w.ok(&["find", &a[..8]]).contains("Retry failed"));
}

#[test]
fn ls_filters_by_column_and_origin_and_hide_takes_a_session_off_it() {
    let (w, a, b) = finder();
    let ls = |args: &[&str]| -> Vec<String> {
        let mut v = vec!["ls", "--json"];
        v.extend_from_slice(args);
        w.json(&v)
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ls(&[]).len(), 2);
    assert_eq!(ls(&["--column", "needs_you"]), vec![a.clone()]);
    assert_eq!(ls(&["--column", "done"]), vec![b.clone()]);
    assert_eq!(ls(&["--origin", "night"]), vec![b.clone()]);
    assert_eq!(ls(&["--limit", "1"]).len(), 1);
    w.ok(&["hide", &a[..8]]);
    assert_eq!(ls(&[]), vec![b.clone()]);
    assert_eq!(ls(&["--all"]).len(), 2);
    w.ok(&["unhide", &a[..8]]);
    assert_eq!(ls(&[]).len(), 2);
    assert!(!w.err(&["ls", "--column", "nowhere"]).is_empty());
}

#[test]
fn show_prints_a_session_as_text_json_and_conversation() {
    let (w, a, _) = finder();
    let text = w.ok(&["show", &a]);
    assert!(text.contains("Retry failed webhook deliveries"), "{text}");
    assert!(text.contains("needs_you"), "{text}");
    assert!(text.contains("claude --resume"), "{text}");
    let j = w.json(&["show", &a[..10], "--json"]);
    assert_eq!(j["id"], a.as_str());
    assert!(w.err(&["show", "ffffffff"]).contains("no session"));
    // a transcript makes the conversation readable; secrets are masked
    let t = w.root.join("claude/projects/-p/t.jsonl");
    std::fs::create_dir_all(t.parent().unwrap()).unwrap();
    std::fs::write(
        &t,
        format!(
            "{}\n{}\n",
            serde_json::json!({"type":"user","timestamp":"2026-11-01T10:00:00.000Z","cwd":"/x","sessionId":a,"message":{"content":"first question"}}),
            serde_json::json!({"type":"assistant","timestamp":"2026-11-01T10:00:05.000Z","cwd":"/x","sessionId":a,"message":{"content":[{"type":"text","text":"the answer uses ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}]}})
        ),
    )
    .unwrap();
    w.db()
        .conn
        .execute(
            "UPDATE session SET transcript_path=?2, transcript_state='present' WHERE id=?1",
            params![a, t.display().to_string()],
        )
        .unwrap();
    let conv = w.ok(&["show", &a, "--conversation", "-n", "5"]);
    assert!(
        conv.contains("first question") && conv.contains("the answer"),
        "{conv}"
    );
    assert!(!conv.contains("ghp_AAAA"), "{conv}");
}

#[test]
fn pick_without_a_screen_lists_ranks_and_prints_the_resume_command() {
    let (w, a, b) = finder();
    let list = w.ok(&["pick", "--filter", "webhook"]);
    assert!(list.contains("Retry failed webhook deliveries"), "{list}");
    assert!(!list.contains("stale"), "{list}");
    let all = w.ok(&["pick", "--filter", "", "--limit", "1", "--no-scan"]);
    assert_eq!(
        all.lines().filter(|l| l.contains("c1e0000")).count(),
        1,
        "{all}"
    );
    let cmd = w.ok(&["pick", "--first", "--print", "stale", "tokens"]);
    assert!(cmd.contains(&format!("claude --resume {b}")), "{cmd}");
    let fork = w.ok(&["pick", "--first", "--print", "--fork", "webhook"]);
    assert!(
        fork.contains("--fork-session") && fork.contains(&a),
        "{fork}"
    );
    assert!(!w
        .err(&["pick", "--first", "--print", "qqqqzzzz"])
        .is_empty());
}

// ------------------------------------------------------------------------------------------- idea

#[test]
fn idea_add_edit_and_ls_keep_what_was_typed_and_clear_what_is_emptied() {
    let w = World::new();
    let dir = w.dir("notes-app");
    let out = w.ok(&[
        "idea",
        "add",
        "Try the new linter",
        "--notes",
        "ask Ana first",
        "--prompt",
        "Run it on the repo",
        "--dir",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.contains("Try the new linter") || out.contains('1'),
        "{out}"
    );
    w.ok(&["idea", "add", "A second one"]);
    let j = w.json(&["idea", "ls", "--json"]);
    assert_eq!(j.as_array().unwrap().len(), 2);
    let first = j
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"] == "Try the new linter")
        .unwrap()
        .clone();
    assert_eq!(first["notes"], "ask Ana first");
    assert_eq!(first["prompt"], "Run it on the repo");
    assert_eq!(first["folder"], dir.display().to_string());
    let id = first["id"].as_i64().unwrap().to_string();

    w.ok(&[
        "idea",
        "edit",
        &id,
        "--title",
        "Try the linter",
        "--notes",
        "",
    ]);
    let after = w.json(&["idea", "ls", "--json"]);
    let edited = after
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"].as_i64().unwrap().to_string() == id)
        .unwrap();
    assert_eq!(edited["title"], "Try the linter");
    assert!(
        edited["notes"].is_null() || edited["notes"] == "",
        "{edited}"
    );
    assert_eq!(edited["prompt"], "Run it on the repo");

    let text = w.ok(&["idea", "ls"]);
    assert!(
        text.contains("Try the linter") && text.contains("A second one"),
        "{text}"
    );
}

#[test]
fn idea_commands_refuse_an_empty_title_an_unknown_idea_and_an_unknown_agent() {
    let w = World::new();
    assert!(w.err(&["idea", "add", "  "]).contains("needs a title"));
    assert!(w
        .err(&["idea", "edit", "99", "--title", "x"])
        .contains("no such idea"));
    assert!(!w
        .err(&["idea", "add", "x", "--agent", "nosuchagent"])
        .is_empty());
    assert!(!w
        .err(&["idea", "add", "x", "--from-session", "ffffffff"])
        .is_empty());
}

// --------------------------------------------------------------------------------------- night

#[test]
fn night_plans_lists_cancels_and_refuses_what_cannot_be_done() {
    let w = World::new();
    let dir = w.dir("notes-app");
    w.ok(&[
        "idea",
        "add",
        "Lint it",
        "--prompt",
        "Run the linter",
        "--dir",
        dir.to_str().unwrap(),
    ]);
    let planned = w.ok(&["night", "add", "idea:1", "--limit", "90m"]);
    assert!(
        planned.contains("planned run 1") && planned.contains("night shift is off"),
        "{planned}"
    );
    let ls = w.ok(&["night", "ls"]);
    assert!(
        ls.contains("planned") && ls.contains("idea:1") && ls.contains("limit"),
        "{ls}"
    );
    let j = w.json(&["night", "ls", "--json"]);
    assert_eq!(j[0]["status"], "planned");
    assert_eq!(j[0]["limit"]["text"], "90 minutes");

    // a session needs a prompt of its own
    let id = w.session(1, "Failing tests", &dir, None, 10);
    assert!(!w.err(&["night", "add", &id[..8]]).is_empty());
    assert!(w
        .ok(&["night", "add", &id[..8], "--prompt", "Fix the tests"])
        .contains("planned run 2"));

    // bring and discard only act on a run that made a worktree or a copy
    assert!(!w.err(&["night", "bring", "1", "--yes"]).is_empty());
    assert!(!w.err(&["night", "discard", "1", "--yes"]).is_empty());
    assert!(!w.err(&["night", "log", "99"]).is_empty());

    w.ok(&["night", "cancel", "1"]);
    assert!(
        !w.err(&["night", "cancel", "1"]).is_empty(),
        "a run cancelled twice"
    );
    assert_eq!(
        w.json(&["night", "ls", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["status"] == "planned")
            .count(),
        1
    );
}

#[test]
fn night_switches_and_settings_are_written_to_the_config_and_checked() {
    let w = World::new();
    assert!(w.ok(&["night", "on"]).to_lowercase().contains("on"));
    assert!(w.ok(&["night", "ls"]).contains("night shift on"));
    w.ok(&["night", "window", "22:30-06:00"]);
    w.ok(&["night", "parallel", "4"]);
    let ls = w.ok(&["night", "ls"]);
    assert!(ls.contains("22:30-06:00") && ls.contains("up to 4"), "{ls}");
    assert!(!w.err(&["night", "window", "22:30-22:30"]).is_empty());
    assert!(!w.err(&["night", "parallel", "9"]).is_empty());
    w.ok(&["night", "off"]);
    assert!(w.ok(&["night", "ls"]).contains("night shift off"));
    let config = std::fs::read_to_string(w.paths().config_path()).unwrap();
    assert!(config.contains("22:30-06:00"), "{config}");
}

// ------------------------------------------------------------------------- export and import

#[test]
fn export_and_import_round_trip_through_standard_streams() {
    let (w, a, b) = finder();
    let pack = w.run(&["export", "--to", "-"], b"");
    assert!(
        pack.status.success(),
        "{}",
        String::from_utf8_lossy(&pack.stderr)
    );
    assert!(pack.stdout.len() > 100, "an empty pack");

    // into a second, empty home: the plan first, then the real thing
    let other = World::new();
    let plan = other.run(&["import", "-", "--dry-run"], &pack.stdout);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    assert_eq!(
        other
            .json(&["ls", "--json", "--all"])
            .as_array()
            .unwrap()
            .len(),
        0,
        "a dry run changed something"
    );
    // without --yes and without a terminal it does not go ahead
    let asked = other.run(&["import", "-"], &pack.stdout);
    assert!(!asked.status.success());
    let done = other.run(&["import", "-", "--yes"], &pack.stdout);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let ids: Vec<String> = other
        .json(&["ls", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect();
    assert!(ids.contains(&a) && ids.contains(&b), "{ids:?}");
    assert!(other
        .ok(&["find", "stale", "tokens"])
        .contains("Fix stale tokens"));
}

#[test]
fn export_of_one_session_writes_a_readable_page_to_standard_output() {
    let (w, a, _) = finder();
    let md = w.ok(&["export", &a, "--md", "--to", "-"]);
    assert!(md.contains("Retry failed webhook deliveries"), "{md}");
    let html = w.ok(&["export", &a, "--to", "-"]);
    assert!(
        html.contains("<html") && html.contains("Retry failed webhook deliveries"),
        "{html}"
    );
}
