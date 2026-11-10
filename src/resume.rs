use crate::config::Config;
use crate::paths::Paths;
pub use crate::platform::terminal::{Mode, Target};
use crate::platform::{self, clipboard};
use anyhow::{Context, Result};
use std::process::{Command, Stdio};

/// What is said when copy mode finds no clipboard tool; the command follows it.
pub const NO_CLIPBOARD: &str = "no clipboard tool; here is the command";

/// Where `[board] resume` sends a command on this machine, worked out once: ask it once per
/// request and use the answer for the label, the plan and the run.
pub fn target(cfg: &Config, paths: &Paths) -> Target {
    let env = platform::terminal::Env::current(paths.home.join("tmp"));
    Target::resolve(&cfg.board.resume, &env)
}

/// A command and what to do with it.
#[derive(Debug, Clone)]
pub struct Plan {
    pub target: Target,
    pub command: String,
    /// AppleScript that would open the iTerm2 tab (iterm mode only).
    pub applescript: Option<String>,
}

impl Plan {
    pub fn mode(&self) -> Mode {
        self.target.mode()
    }
}

/// What iTerm types, and what a `.command` file holds: the command without control characters
/// (a newline stays).
pub(crate) fn typed(command: &str) -> String {
    command
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

/// The AppleScript that opens an iTerm2 tab and types `command`; control characters are dropped.
pub fn applescript_for(command: &str) -> String {
    let esc = typed(command).replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "tell application \"iTerm2\"\n  activate\n  if (count of windows) = 0 then\n    create window with default profile\n  else\n    tell current window to create tab with default profile\n  end if\n  tell current session of current window to write text \"{esc}\"\nend tell"
    )
}

/// A plan for a command line: where it goes and, for iTerm, the AppleScript that opens the tab.
pub fn plan_command(command: String, target: &Target) -> Plan {
    let applescript = (target.mode() == Mode::Iterm).then(|| applescript_for(&command));
    Plan {
        target: target.clone(),
        command,
        applescript,
    }
}

/// Run a plan: open an iTerm2 tab or a terminal window, or put the command on the clipboard.
/// Returns a one-line note for the user.
pub fn execute(p: &Plan) -> Result<String> {
    match p.mode() {
        Mode::Iterm => {
            let script = p
                .applescript
                .clone()
                .unwrap_or_else(|| applescript_for(&p.command));
            let out = platform::run_applescript(&script)?;
            anyhow::ensure!(
                out.status.success(),
                "iTerm did not open: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            Ok("opened in iTerm".into())
        }
        Mode::Terminal => p.target.open_terminal(&p.command),
        Mode::Copy => Ok(copy_note(clipboard::copy(&p.command), &p.command)),
    }
}

/// What copy mode says: the command is on the clipboard, or here it is because no tool took it.
fn copy_note(copied: bool, command: &str) -> String {
    if copied {
        "command copied to the clipboard".into()
    } else {
        format!("{NO_CLIPBOARD}: {command}")
    }
}

/// Run a shell command and return its stdout and stderr; a failure carries that text.
pub fn run_shell(command: &str) -> Result<String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::null())
        .output()
        .context("running sh")?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    anyhow::ensure!(out.status.success(), "{}", text.trim());
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::Invocation;
    use crate::platform::terminal::Env;
    use crate::platform::test_support::{script, stub};
    use std::path::Path;

    fn line(cwd: &str, id: &str) -> String {
        Invocation::new("claude", &["--resume", id]).shell_line(cwd)
    }

    /// A machine whose only programs are in `bin`; it has no iTerm2.
    fn machine(macos: bool, bin: &Path) -> Env {
        Env {
            macos,
            path: bin.into(),
            app_dirs: vec![],
            display: true,
            terminal_var: None,
            tmp: bin.join("tmp"),
        }
    }

    fn mac_iterm() -> Target {
        Target::resolve("iterm", &machine(true, Path::new("")))
    }

    #[test]
    fn iterm_script_escapes() {
        let p = plan_command(line("/tmp/a b", "abcdef12"), &mac_iterm());
        assert_eq!(p.command, "cd '/tmp/a b' && claude --resume abcdef12");
        let s = p.applescript.unwrap_or_default();
        assert!(s.contains("write text \"cd '/tmp/a b' && claude --resume abcdef12\""));
        assert!(applescript_for("echo \"x\"").contains("echo \\\"x\\\""));
        assert!(plan_command(line("/x", "a"), &Target::copy())
            .applescript
            .is_none());
    }

    /// Undo the AppleScript string escapes: what iTerm would type.
    fn unescape(script: &str) -> String {
        let a = script.find("write text \"").unwrap() + "write text \"".len();
        let body = &script[a..script.len() - "\"\nend tell".len()];
        let mut out = String::new();
        let mut it = body.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                out.push(it.next().unwrap());
            } else {
                assert_ne!(c, '"', "unescaped quote ends the AppleScript string");
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn a_new_config_resumes_by_itself_on_every_os() {
        assert_eq!(platform::default_resume(), "auto");
        let cfg = Config::parse(&crate::config::default_toml()).unwrap();
        assert_eq!(cfg.board.resume, "auto");
        assert_eq!(Config::default().board.resume, "auto");
        let on = |toml: &str| Config::parse(toml).unwrap().board.resume;
        // configs written before `auto` keep their meaning
        assert_eq!(on("[board]\nresume = \"iterm\""), "iterm");
        assert_eq!(on("[board]\nresume = \"copy\""), "copy");
        assert_eq!(on("[board]\nenabled = true"), "auto");
    }

    #[test]
    fn the_plan_carries_what_the_target_resolved_to() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: t.path().join("eye"),
        };
        let cfg =
            |setting: &str| Config::parse(&format!("[board]\nresume = \"{setting}\"")).unwrap();
        let copy = target(&cfg("copy"), &paths);
        assert_eq!(copy.mode(), Mode::Copy);
        let p = plan_command(line("/x", "a"), &copy);
        assert_eq!((p.mode(), p.target.terminal()), (Mode::Copy, None));
        // whatever this machine is, the answer is one of the three and says what it is
        let here = target(&cfg("auto"), &paths);
        let p = plan_command(line("/x", "a"), &here);
        assert_eq!(p.applescript.is_some(), p.mode() == Mode::Iterm);
        assert_eq!(p.target.terminal().is_some(), here.terminal().is_some());
        assert!(["iterm", "terminal", "copy"].contains(&p.mode().as_str()));
    }

    #[test]
    fn a_linux_terminal_plan_runs_the_terminal() {
        let t = tempfile::tempdir().unwrap();
        script(
            t.path(),
            "xdg-terminal-exec",
            "printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"",
        );
        let target = Target::resolve("auto", &machine(false, t.path()));
        let p = plan_command(line("/tmp/a b", "abcdef12"), &target);
        assert_eq!(
            (p.mode(), p.target.terminal()),
            (Mode::Terminal, Some("xdg-terminal-exec"))
        );
        assert!(p.applescript.is_none());
        assert_eq!(
            execute(&p).unwrap(),
            "opened in a new terminal (xdg-terminal-exec)"
        );
        let args = std::fs::read_to_string(t.path().join("args")).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            ["sh", "-c", "cd '/tmp/a b' && claude --resume abcdef12"]
        );
    }

    #[test]
    fn a_mac_terminal_plan_opens_a_command_file() {
        let t = tempfile::tempdir().unwrap();
        stub(t.path(), "open", 0);
        let target = Target::resolve("auto", &machine(true, t.path()));
        let p = plan_command(line("/tmp/a b", "abcdef12"), &target);
        assert_eq!((p.mode(), p.target.terminal()), (Mode::Terminal, None));
        assert!(p.applescript.is_none());
        assert_eq!(execute(&p).unwrap(), "opened in a new terminal window");
        let opened = std::fs::read_to_string(t.path().join("open.log")).unwrap();
        assert!(std::fs::read_to_string(opened.trim())
            .unwrap()
            .contains("cd '/tmp/a b' && claude --resume abcdef12"));
    }

    #[test]
    fn copy_mode_hands_the_command_over_when_nothing_took_it() {
        let cmd = "cd '/x' && claude --resume a";
        assert_eq!(copy_note(true, cmd), "command copied to the clipboard");
        assert_eq!(
            copy_note(false, cmd),
            "no clipboard tool; here is the command: cd '/x' && claude --resume a"
        );
    }

    #[test]
    fn nasty_strings_round_trip() {
        let nasty = "a'b\"c\\d $(touch /tmp/x) `id` ; & \u{1b}[31m \u{3} é";
        let p = plan_command(
            line(nasty, "30194328-aaaa-4bbb-8ccc-0123456789ab"),
            &mac_iterm(),
        );
        let typed = unescape(&p.applescript.unwrap());
        let want: String = p.command.chars().filter(|c| !c.is_control()).collect();
        assert_eq!(typed, want);
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", crate::util::shell_quote(nasty)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), nasty);
        let p = plan_command(line("/x", "x; touch /tmp/pwn"), &Target::copy());
        assert!(p.command.ends_with("--resume 'x; touch /tmp/pwn'"));
    }
}
