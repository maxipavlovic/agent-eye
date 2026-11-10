//! What `agent-eye install`, `uninstall` and `board on|off` print: a line for each area, home
//! paths as `~`, file diffs only when asked for, and the Codex trust step once, at the end.
//! Everything runs against a temp HOME with `--no-service`; no service manager is run.

use std::process::{Command, Stdio};

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

/// A HOME with Claude Code's and Codex's folders in it, or only some of them.
struct Machine {
    tmp: tempfile::TempDir,
}

const DAEMON: &str = if cfg!(target_os = "macos") {
    "dev.agent-eye.daemon.plist"
} else {
    "agent-eye.service"
};

impl Machine {
    fn new(claude: bool, codex: bool) -> Machine {
        let m = Machine {
            tmp: tempfile::tempdir().unwrap(),
        };
        for (dir, there) in [(".claude", claude), (".codex", codex)] {
            if there {
                std::fs::create_dir_all(m.home().join(dir)).unwrap();
            }
        }
        m
    }

    fn home(&self) -> std::path::PathBuf {
        self.tmp.path().join("home")
    }

    /// Run `agent-eye <args>` with the service files in `~/.svc` and the binary in `~/.local/bin`.
    fn run(&self, args: &[&str]) -> Out {
        let home = self.home();
        std::fs::create_dir_all(&home).unwrap();
        let mut all: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        if matches!(args[0], "install" | "uninstall") {
            for (flag, value) in [
                ("--bin-dir", home.join(".local/bin")),
                ("--service-dir", home.join(".svc")),
            ] {
                all.push(flag.into());
                all.push(value.to_string_lossy().into_owned());
            }
        }
        all.push("--no-service".into());
        let o = Command::new(env!("CARGO_BIN_EXE_agent-eye"))
            .env("HOME", &home)
            .env_remove("EYE_HOME")
            .env_remove("EYE_CLAUDE_DIR")
            .env_remove("EYE_CLAUDE_BIN")
            .env_remove("EYE_CODEX_BIN")
            .env_remove("CODEX_HOME")
            // no agent program on PATH: an agent is there when its folder is
            .env("PATH", "/usr/bin:/bin")
            .env("NO_COLOR", "1")
            .args(&all)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        Out {
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
            code: o.status.code().unwrap_or(-1),
        }
    }

    /// The report with the dates of backups made out and the home folder named `/HOME`, so what
    /// is left is the same on every machine.
    fn tidy(&self, out: &Out) -> String {
        let home = self.home().to_string_lossy().into_owned();
        let dated = regex::Regex::new(r"eye-backup-\d{8}-\d{6}").unwrap();
        dated
            .replace_all(&out.stdout.replace(&home, "/HOME"), "eye-backup-DATE")
            .into_owned()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.home().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

/// Another tool's hook in a hooks file: install keeps it, and backs the file up first.
const OTHER_TOOLS_HOOKS: &str = r#"{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/notify/bin/notify stop"
          }
        ]
      }
    ]
  }
}
"#;

/// The rows of a report that are the same on every OS (the daemon's differs by service manager).
fn without_daemon(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with("  daemon "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_first_install_is_one_line_for_each_area_with_home_paths_as_tilde() {
    let m = Machine::new(true, true);
    let out = m.run(&["install"]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert!(out.stderr.is_empty(), "nothing on stderr: {}", out.stderr);
    let text = out.stdout.clone();
    // the commands to run by hand are the one place a path is spelled out: they have to work
    assert!(
        !without_daemon(&text).contains(&m.home().to_string_lossy().into_owned()),
        "no home folder spelled out: {text}"
    );
    assert_eq!(
        without_daemon(&text),
        "agent-eye install
  agents  Claude Code, Codex CLI (both enabled in ~/.agent-eye/config.toml)
  binary  ~/.local/bin/agent-eye (installed), eye -> agent-eye (linked)
  claude  hooks added (8 events) to ~/.claude/settings.json, skill added
  codex   hooks added (6 events) to ~/.codex/hooks.json, skill added
  board   off (turn it on with `agent-eye board on`)

Next: start Codex and run /hooks once to approve eye's hooks (Codex asks again when a hook changes)"
    );
    let daemon = text.lines().find(|l| l.starts_with("  daemon ")).unwrap();
    assert!(
        daemon.starts_with(&format!(
            "  daemon  added ~/.svc/{DAEMON}; not started (--no-service). To start it run `"
        )),
        "{daemon}"
    );
    // a normal install shows no file contents
    assert!(!text.contains("+    1:"), "{text}");
}

#[test]
fn installing_again_says_what_is_already_in_place_and_asks_for_nothing() {
    let m = Machine::new(true, true);
    m.run(&["install"]);
    let out = m.run(&["install"]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert_eq!(
        without_daemon(&out.stdout),
        "agent-eye install
  agents  Claude Code, Codex CLI (from ~/.agent-eye/config.toml)
  binary  ~/.local/bin/agent-eye (unchanged), eye -> agent-eye
  claude  hooks ok (8 events), skill ok
  codex   hooks ok (6 events), skill ok
  board   off (turn it on with `agent-eye board on`)"
    );
    assert!(!out.stdout.contains("Next:"), "{}", out.stdout);
    let daemon = out
        .stdout
        .lines()
        .find(|l| l.starts_with("  daemon "))
        .unwrap();
    assert!(
        daemon.starts_with(&format!("  daemon  ~/.svc/{DAEMON} unchanged; not started")),
        "{daemon}"
    );
}

#[test]
fn a_dry_run_says_would_and_shows_the_diff_under_the_line_it_belongs_to() {
    let m = Machine::new(false, true);
    m.write(".codex/hooks.json", OTHER_TOOLS_HOOKS);
    let out = m.run(&["install", "--dry-run"]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let text = m.tidy(&out);
    assert!(
        text.starts_with(
            "agent-eye install (dry run: nothing was changed)
  agents  Codex CLI (would enable it in ~/.agent-eye/config.toml)
  binary  ~/.local/bin/agent-eye (would install), eye -> agent-eye (would link)
  codex   would add hooks (6 events) to ~/.codex/hooks.json, would add skill
"
        ),
        "{text}"
    );
    // the diff is indented under the codex line: lines added, none of the user's own removed
    let after_codex = text.split("  codex   ").nth(1).unwrap();
    let diff: Vec<&str> = after_codex
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with("            "))
        .collect();
    assert!(diff.len() > 20, "{text}");
    assert!(
        diff.iter().all(|l| l.starts_with("            + ")),
        "{diff:?}"
    );
    assert!(
        diff.iter()
            .any(|l| l.contains("hook PostToolUse --agent codex")),
        "{diff:?}"
    );
    assert!(text.contains("\n  daemon  would add "), "{text}");
    assert_eq!(text.matches("Next: ").count(), 1, "{text}");
    // and nothing was written
    assert_eq!(
        std::fs::read_to_string(m.home().join(".codex/hooks.json")).unwrap(),
        OTHER_TOOLS_HOOKS
    );
    assert!(!m.home().join(".local").exists() && !m.home().join(".agent-eye").exists());
}

#[test]
fn verbose_shows_the_diff_of_a_real_install_and_nothing_else_does() {
    let m = Machine::new(false, true);
    m.write(".codex/hooks.json", OTHER_TOOLS_HOOKS);
    let out = m.run(&["install", "--verbose"]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    let text = m.tidy(&out);
    assert!(text.contains("\n            + "), "{text}");
    assert!(text.contains("  codex   hooks added (6 events) to ~/.codex/hooks.json (backup ~/.codex/hooks.json.eye-backup-DATE), skill added\n"), "{text}");
    let plain = Machine::new(false, true);
    plain.write(".codex/hooks.json", OTHER_TOOLS_HOOKS);
    let out = plain.run(&["install"]);
    assert!(!out.stdout.contains("\n            + "), "{}", out.stdout);
}

#[test]
fn a_codex_hooks_change_tells_you_once_what_to_trust_and_a_plain_install_does_not() {
    let m = Machine::new(true, true);
    m.write(".codex/hooks.json", OTHER_TOOLS_HOOKS);
    let out = m.run(&["install"]);
    let text = m.tidy(&out);
    assert_eq!(
        text.matches("Next: start Codex and run /hooks once to approve eye's hooks")
            .count(),
        1,
        "{text}"
    );
    assert!(
        text.trim_end()
            .ends_with("(Codex asks again when a hook changes)"),
        "it is the last line: {text}"
    );
    // the other tool's hook is still there
    let hooks = std::fs::read_to_string(m.home().join(".codex/hooks.json")).unwrap();
    assert!(hooks.contains("/opt/notify/bin/notify stop"));
    // Claude Code alone: no Codex, no trust step
    let claude = Machine::new(true, false);
    let out = claude.run(&["install"]);
    assert!(
        !out.stdout.contains("Next:") && !out.stdout.contains("Codex"),
        "{}",
        out.stdout
    );
}

#[test]
fn an_uninstall_prints_the_same_kind_of_summary() {
    let m = Machine::new(true, true);
    m.run(&["install"]);
    let out = m.run(&["uninstall"]);
    assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
    assert!(out.stderr.is_empty(), "{}", out.stderr);
    let text = m.tidy(&out);
    assert_eq!(
        without_daemon(&text),
        "agent-eye uninstall
  binary  removed ~/.local/bin/agent-eye, eye
  claude  hooks removed (8 entries) from ~/.claude/settings.json (backup ~/.claude/settings.json.eye-backup-DATE), skill removed
  codex   hooks removed (6 entries) from ~/.codex/hooks.json (backup ~/.codex/hooks.json.eye-backup-DATE), skill removed
  data    kept ~/.agent-eye (use --purge to delete it)"
    );
    let daemon = text.lines().find(|l| l.starts_with("  daemon ")).unwrap();
    assert!(
        daemon.contains("not stopped (--no-service). To stop it run `")
            && daemon.contains(&format!(", removed ~/.svc/{DAEMON}")),
        "{daemon}"
    );
    // a dry run says what it would do
    let again = Machine::new(true, true);
    again.run(&["install"]);
    let dry = again.run(&["uninstall", "--dry-run", "--purge"]);
    let text = again.tidy(&dry);
    assert!(
        text.starts_with("agent-eye uninstall (dry run: nothing was changed)\n"),
        "{text}"
    );
    for row in [
        "  binary  would remove ~/.local/bin/agent-eye, eye",
        "  claude  would remove hooks (8 entries) from ~/.claude/settings.json, would remove skill",
        "  codex   would remove hooks (6 entries) from ~/.codex/hooks.json, would remove skill",
        "  data    would delete ~/.agent-eye",
    ] {
        assert!(text.lines().any(|l| l == row), "{row}: {text}");
    }
    // the diff of a file that changes is under its line
    assert!(text.contains("\n            - "), "{text}");
    assert!(
        again.home().join(".agent-eye").exists(),
        "nothing was deleted"
    );
}

#[test]
fn board_on_and_off_print_two_short_lines() {
    let m = Machine::new(true, false);
    let on = m.run(&["board", "on"]);
    assert_eq!(on.code, 0, "{}{}", on.stdout, on.stderr);
    let lines: Vec<&str> = on.stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{}", on.stdout);
    assert_eq!(lines[0], "agent-eye board on");
    assert!(
        lines[1].starts_with("  board   on: http://127.0.0.1:7420/?t="),
        "{}",
        lines[1]
    );
    assert_eq!(
        lines[2],
        "  daemon  not restarted (--no-service); restart the daemon to apply"
    );
    let off = m.run(&["board", "off"]);
    assert_eq!(
        off.stdout,
        "agent-eye board off\n  board   off\n  daemon  not restarted (--no-service); restart the daemon to apply\n"
    );
}
