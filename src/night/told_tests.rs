//! What a night run is told, and what it finds: the preamble of an idea run, the messages of the
//! barrier's hooks, and the `eye` on its `PATH`.

use super::tests::{call, opencode_world, repo, run_of};
use super::*;
use crate::loose::System;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const AGENTS: [&str; 4] = ["claude", "codex", "agy", "opencode"];

/// An idea run of a repo at `/Users/me/app` (its base commit is known).
fn repo_run(agent: &str) -> Run {
    Run {
        folder: "/Users/me/app".into(),
        base_ref: Some("abc123".into()),
        ..run_of(agent, "idea")
    }
}

/// The prompt the agent is started with: after `-p` for Claude and agy, the last argument for the
/// others.
fn sent(agent: &str, run: &Run) -> String {
    let s = launch::build(
        &Config::default(),
        run,
        agent,
        Path::new("/w/run/wt"),
        Some("u"),
        None,
    );
    if s.args[0] == "-p" {
        s.args[1].clone()
    } else {
        s.args.last().unwrap().clone()
    }
}

// ---------- the preamble of an idea run ----------

#[test]
fn an_idea_run_is_told_where_it_works_whatever_the_agent() {
    for agent in AGENTS {
        let told = sent(agent, &repo_run(agent));
        for want in [
            "you work in /w/run/wt, a checkout of the last commit of /Users/me/app",
            "Files that are not committed there (untracked notes, uncommitted edits) are not in it",
            "You may read files in /Users/me/app but change nothing there",
            "If something this task needs is missing, stop and say so",
            "Do not push anything",
        ] {
            assert!(told.contains(want), "{agent}: {want}\n{told}");
        }
        assert!(told.ends_with("\n\nmake it better"), "{agent}: {told}");
        // one wording for all of them
        for name in ["Claude", "Codex", "agy", "OpenCode", "Antigravity"] {
            assert!(!told.contains(name), "{agent} names {name}: {told}");
        }
    }
}

#[test]
fn what_is_refused_is_told_the_way_the_agent_meets_it() {
    // Claude's hook refuses a shell command that names the folder; its file tool still reads
    let told = sent("claude", &repo_run("claude"));
    assert!(
        told.contains("change nothing there; if a shell command that names it is refused, read the file with your file-reading tool. If something"),
        "{told}"
    );
    // OpenCode's rules refuse its file tools outside the run folder; its shell reads anything
    let told = sent("opencode", &repo_run("opencode"));
    assert!(
        told.contains("change nothing there; your file tools refuse a path outside /w/run/wt, so read a file there with a shell command. If something"),
        "{told}"
    );
    assert!(!told.contains("file-reading tool"), "{told}");
    // Codex and agy refuse nothing that names the folder: nothing to say about it
    for agent in ["codex", "agy"] {
        let told = sent(agent, &repo_run(agent));
        assert!(
            told.contains("change nothing there. If something"),
            "{agent}: {told}"
        );
        assert!(!told.contains("refused"), "{agent}: {told}");
    }
}

#[test]
fn the_folder_it_is_told_has_no_trailing_slash() {
    // a run planned at the top of its repo starts in `wt/` (the worktree joined with nothing)
    let s = launch::build(
        &Config::default(),
        &repo_run("codex"),
        "codex",
        Path::new("/w/run/wt/"),
        None,
        None,
    );
    let told = s.args.last().unwrap();
    assert!(told.contains("you work in /w/run/wt, a checkout"), "{told}");
}

#[test]
fn an_idea_run_of_a_folder_that_is_not_a_repo_is_told_it_is_a_copy() {
    let run = Run {
        base_ref: None,
        ..repo_run("claude")
    };
    let told = sent("claude", &run);
    assert!(
        told.contains(
            "a copy of /Users/me/app made when the run started, without node_modules, .git, target, .venv, dist"
        ),
        "{told}"
    );
    assert!(
        !told.contains("last commit") && !told.contains("not committed"),
        "{told}"
    );
    assert!(told.contains("You may read files in /Users/me/app but change nothing there"));
}

#[test]
fn a_continued_session_keeps_its_own_note() {
    let run = Run {
        folder: "/Users/me/app".into(),
        base_ref: Some("abc123".into()),
        ..run_of("claude", "session")
    };
    let told = sent("claude", &run);
    assert_eq!(
        told,
        "Night run: you are continuing this session in an isolated copy of the project at /w/run/wt. \
         The earlier conversation mentions paths in the original folder (/Users/me/app); use the same files \
         under /w/run/wt instead and change nothing in /Users/me/app. Do not push anything.\n\nmake it better"
    );
}

/// The opencode stand-in of `opencode_world` prints the arguments of the run it was given.
fn wait_for(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("{} did not appear", path.display());
}

#[test]
fn a_started_run_of_a_repo_is_told_its_folder_is_the_last_commit() {
    let (tmp, run, paths, cfg, agents) = opencode_world("ok");
    let db = Db::open_memory().unwrap();
    launch::start(&System::new(), &db, &paths, &cfg, &agents, &run).unwrap();
    let ran = tmp.path().join("bin/ran");
    wait_for(&ran);
    let told = std::fs::read_to_string(&ran).unwrap();
    assert!(
        told.contains(&format!("the last commit of {}", run.folder)),
        "{told}"
    );
    assert!(told.contains("tidy up"), "{told}");
}

// ---------- the messages of the barrier's hooks ----------

/// Run a hook command on one tool call: its exit code and what it said on stderr.
fn hook_output(cmd: &str, input: &str) -> (i32, String) {
    use std::io::Write;
    let mut c = Command::new("sh")
        .args(["-c", cmd])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8_lossy(&o.stderr).trim().to_string(),
    )
}

#[test]
fn the_bash_hook_says_a_call_that_names_the_folder_is_refused_and_where_to_read_instead() {
    let guard = vec![
        PathBuf::from("/Users/me/app"),
        PathBuf::from("/Users/me/lib"),
    ];
    let bash = barrier::hook_command(&guard, true).unwrap();
    for c in ["ls /Users/me/app", "cat /Users/me/lib/notes.md"] {
        let (code, said) = hook_output(&bash, &call("command", c));
        assert_eq!(code, 2, "{c}");
        assert_eq!(
            said,
            "night run: a shell call that names the original folder (/Users/me/app, /Users/me/lib), in its \
             command or its description, is refused; it belongs to you, not to the run; work in the run \
             folder, and read a file from the original with the file-reading tool (Read)",
            "{c}"
        );
    }
    // what is allowed says nothing
    assert_eq!(
        hook_output(&bash, &call("command", "ls /w/run/wt")),
        (0, String::new())
    );
}

/// What the Bash hook does with one command, as Claude Code hands it over: JSON on stdin, so a
/// quote, newline or tab in the command arrives escaped.
fn bash_hook_exit(command: &str) -> i32 {
    let guard = vec![PathBuf::from("/Users/me/app")];
    let bash = barrier::hook_command(&guard, true).unwrap();
    hook_output(&bash, &call("command", command)).0
}

#[test]
fn the_bash_hook_refuses_the_folder_in_every_way_a_json_command_can_spell_it() {
    for c in [
        "ls /Users/me/app",
        "cat /Users/me/app/notes.md",
        "ls '/Users/me/app'",
        "ls \"/Users/me/app\"",
        "cat \"/Users/me/app/notes.md\"",
        "cd /Users/me/app\nls",
        "cd \"/Users/me/app\"\ntouch x",
        "cd \"/Users/me/app\" && touch x",
        "ls /Users/me/app\t-la",
        "ls /Users/me/app\r\nls",
        "PATH=/Users/me/app:$PATH make",
        "echo hi >/Users/me/app/x",
        "rsync -a wt/ /Users/me/app/",
        "(cd /Users/me/app)",
    ] {
        assert_eq!(bash_hook_exit(c), 2, "{c:?}");
    }
    // another folder, and the run's own, stay open
    for c in [
        "ls /Users/me/application",
        "ls \"/Users/me/application\"",
        "cd /Users/me/app-two\nls",
        "ls /w/run/wt",
    ] {
        assert_eq!(bash_hook_exit(c), 0, "{c:?}");
    }
}

#[test]
fn the_bash_hook_also_refuses_a_call_whose_description_names_the_folder() {
    // the hook reads the call as text, not as JSON, so this is refused too: the message says so
    let guard = vec![PathBuf::from("/Users/me/app")];
    let bash = barrier::hook_command(&guard, true).unwrap();
    let call =
        serde_json::json!({"tool_input": {"command": "ls", "description": "list /Users/me/app"}})
            .to_string();
    let (code, said) = hook_output(&bash, &call);
    assert_eq!(code, 2);
    assert!(said.contains("in its command or its description"), "{said}");
}

#[test]
fn the_file_hook_says_the_file_tools_write_only_in_the_run_folder() {
    let guard = vec![PathBuf::from("/Users/me/app")];
    let files = barrier::hook_command(&guard, false).unwrap();
    for p in [
        "/Users/me/app/src/a.rs",
        "/w/run/wt/../../app/x",
        "../app/x",
    ] {
        let (code, said) = hook_output(&files, &call("file_path", p));
        assert_eq!(code, 2, "{p}");
        assert_eq!(
            said,
            "night run: the file tools may write only in the run folder, not in the original folder \
             (/Users/me/app) and not through a path that climbs out of the run folder with ..",
            "{p}"
        );
    }
    assert_eq!(
        hook_output(&files, &call("file_path", "/w/run/wt/a.rs")),
        (0, String::new())
    );
}

#[test]
fn the_hooks_leave_reading_alone() {
    // the Read tool is not hooked, which is where the message sends a run
    let b = barrier::plan(
        Path::new("/w/run/wt"),
        &[PathBuf::from("/Users/me/app")],
        true,
    );
    let v: serde_json::Value =
        serde_json::from_str(&barrier::settings(&b, &launch::DENY, &[], true)).unwrap();
    let matchers: Vec<&str> = v["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["matcher"].as_str().unwrap())
        .collect();
    assert_eq!(matchers, ["Write|Edit|MultiEdit|NotebookEdit", "Bash"]);
}

// ---------- the eye a run finds first ----------

#[test]
fn every_run_starts_with_eyes_own_folder_first_on_its_path() {
    let tmp = tempfile::tempdir().unwrap();
    let probe = tmp.path().join("probe");
    std::fs::write(
        &probe,
        "#!/bin/sh\necho \"$PATH\" > path.txt\ncommand -v eye > eye.txt\n",
    )
    .unwrap();
    std::fs::set_permissions(&probe, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let bin = tmp.path().join("home/bin");
    bin::refresh(&bin).unwrap();
    let spec = launch::Spec {
        program: probe.to_string_lossy().into(),
        args: vec![],
        cwd: tmp.path().into(),
        session_id: None,
    };
    let exit = tmp.path().join("exit");
    launch::spawn(&spec, &bin, &tmp.path().join("log"), &exit).unwrap();
    wait_for(&exit);
    let path = std::fs::read_to_string(tmp.path().join("path.txt")).unwrap();
    assert!(path.starts_with(&format!("{}:", bin.display())), "{path}");
    let eye = std::fs::read_to_string(tmp.path().join("eye.txt")).unwrap();
    assert_eq!(eye.trim(), bin.join("eye").to_string_lossy());
}

#[test]
fn a_started_run_gets_the_folder_and_its_links_made_before_it_starts() {
    let (tmp, run, paths, cfg, agents) = opencode_world("ok");
    crate::platform::test_support::script(
        &tmp.path().join("bin"),
        "opencode",
        r#"d="$(dirname "$0")"
case "$1" in
debug) echo '[{"type":"document","info":{"mcp":{"servers":{}}}}]' ;;
*) echo "$PATH" > "$d/path"; command -v eye > "$d/eye" ;;
esac"#,
    );
    let db = Db::open_memory().unwrap();
    launch::start(&System::new(), &db, &paths, &cfg, &agents, &run).unwrap();
    let eye = tmp.path().join("bin/eye");
    wait_for(&eye);
    let home_bin = bin::dir(&paths);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("bin/path"))
            .unwrap()
            .split(':')
            .next(),
        home_bin.to_str()
    );
    assert_eq!(
        std::fs::read_to_string(&eye).unwrap().trim(),
        home_bin.join("eye").to_string_lossy()
    );
    assert_eq!(
        std::fs::read_link(home_bin.join("eye")).unwrap(),
        std::env::current_exe().unwrap()
    );
}

#[test]
fn the_sandbox_may_read_eyes_links_and_the_binary_they_point_to() {
    let mut b = barrier::plan(
        Path::new("/w/run/wt"),
        &[PathBuf::from("/Users/me/app")],
        true,
    );
    let filesystem = |b: &barrier::Barrier| -> serde_json::Value {
        let v: serde_json::Value =
            serde_json::from_str(&barrier::settings(b, &launch::DENY, &[], true)).unwrap();
        v["sandbox"]["filesystem"].clone()
    };
    // nothing to open: the key stays out of the settings
    assert!(filesystem(&b).get("allowRead").is_none());
    b.allow_reading(Path::new("/home/me/.agent-eye/bin"));
    b.allow_reading(Path::new("/home/me/.agent-eye/bin"));
    assert_eq!(
        filesystem(&b)["allowRead"],
        serde_json::json!(["/home/me/.agent-eye/bin"])
    );
}

#[test]
fn a_started_claude_run_lets_its_sandbox_read_what_its_path_points_to() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("proj");
    repo(&orig);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // the settings are the one argument that is JSON
    crate::platform::test_support::script(
        &bin,
        "claude",
        r#"for a in "$@"; do case "$a" in '{'*) printf '%s' "$a" > "$(dirname "$0")/settings.json" ;; esac; done"#,
    );
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    let cfg = Config::parse("[night]\nenabled = true\nallow_unsandboxed = true\n").unwrap();
    let exe = bin.join("claude");
    let agents = Agents::from_vec(vec![std::sync::Arc::new(
        crate::agents::claude::Claude::new(tmp.path().join("claude"))
            .with_executable(exe.to_str().unwrap()),
    )]);
    let run = Run {
        id: 7,
        kind: "idea".into(),
        agent: "claude".into(),
        prompt: "tidy up".into(),
        folder: orig.to_string_lossy().into(),
        night: "2026-11-21".into(),
        ..Run::default()
    };
    let db = Db::open_memory().unwrap();
    launch::start(&System::new(), &db, &paths, &cfg, &agents, &run).unwrap();
    wait_for(&super::run_dir(&paths, 7).join("exit"));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bin.join("settings.json")).unwrap()).unwrap();
    // the settings carry the sandbox exactly when this machine has one
    assert_eq!(v.get("sandbox").is_some(), barrier::available().is_ok());
    if let Some(sandbox) = v.get("sandbox") {
        let read: Vec<&str> = sandbox["filesystem"]["allowRead"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p.as_str())
            .collect();
        assert!(
            read.contains(&bin::dir(&paths).to_str().unwrap()),
            "{read:?}"
        );
        assert!(
            read.contains(&std::env::current_exe().unwrap().to_str().unwrap()),
            "{read:?}"
        );
    }
}
