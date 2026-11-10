use crate::config::Config;
use crate::platform::{self, clipboard};
use anyhow::{Context, Result};
use std::process::{Command, Stdio};

/// What is said when copy mode finds no clipboard tool; the command follows it.
pub const NO_CLIPBOARD: &str = "no clipboard tool; here is the command";

/// Where a resume command goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Iterm,
    Copy,
}

impl Mode {
    /// The mode `[board] resume` asks for. Without a terminal eye can open a tab in (it is iTerm2
    /// on macOS only), the command is copied whatever the setting says.
    pub fn from_config(cfg: &Config) -> Mode {
        Mode::choose(&cfg.board.resume, platform::can_open_terminal_tab())
    }

    fn choose(setting: &str, can_open_tab: bool) -> Mode {
        if setting.eq_ignore_ascii_case("copy") || !can_open_tab {
            Mode::Copy
        } else {
            Mode::Iterm
        }
    }
    /// The name used in the API (`iterm` or `copy`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Iterm => "iterm",
            Mode::Copy => "copy",
        }
    }
}

/// A command and what to do with it.
#[derive(Debug, Clone)]
pub struct Plan {
    pub mode: Mode,
    pub command: String,
    /// AppleScript that would open the iTerm2 tab (iterm mode only).
    pub applescript: Option<String>,
}

/// The AppleScript that opens an iTerm2 tab and types `command`; control characters are dropped.
pub fn applescript_for(command: &str) -> String {
    // Control characters other than newline (ESC, ^C, ^D, CR...) would be typed into the
    // terminal as keystrokes; drop them. Backslash and quote are the only AppleScript escapes.
    let esc: String = command
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!(
        "tell application \"iTerm2\"\n  activate\n  if (count of windows) = 0 then\n    create window with default profile\n  else\n    tell current window to create tab with default profile\n  end if\n  tell current session of current window to write text \"{esc}\"\nend tell"
    )
}

/// A plan for a command line: where it goes and, for iTerm, the AppleScript that opens the tab.
pub fn plan_command(command: String, mode: Mode) -> Plan {
    let applescript = (mode == Mode::Iterm).then(|| applescript_for(&command));
    Plan {
        mode,
        command,
        applescript,
    }
}

/// Run a plan: open an iTerm2 tab (iterm mode) or put the command on the clipboard (copy mode).
/// Returns a one-line note for the user.
pub fn execute(p: &Plan) -> Result<String> {
    match (&p.mode, &p.applescript) {
        (Mode::Iterm, Some(script)) => {
            let out = platform::run_applescript(script)?;
            anyhow::ensure!(
                out.status.success(),
                "iTerm did not open: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            Ok("opened in iTerm".into())
        }
        _ => Ok(copy_note(clipboard::copy(&p.command), &p.command)),
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
    fn line(cwd: &str, id: &str) -> String {
        Invocation::new("claude", &["--resume", id]).shell_line(cwd)
    }

    #[test]
    fn iterm_script_escapes() {
        let p = plan_command(line("/tmp/a b", "abcdef12"), Mode::Iterm);
        assert_eq!(p.command, "cd '/tmp/a b' && claude --resume abcdef12");
        let s = p.applescript.unwrap_or_default();
        assert!(s.contains("write text \"cd '/tmp/a b' && claude --resume abcdef12\""));
        assert!(applescript_for("echo \"x\"").contains("echo \\\"x\\\""));
        assert!(plan_command(line("/x", "a"), Mode::Copy)
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
    fn only_a_terminal_that_can_open_a_tab_gets_one() {
        assert_eq!(Mode::choose("iterm", true), Mode::Iterm);
        assert_eq!(Mode::choose("ITERM", true), Mode::Iterm);
        assert_eq!(Mode::choose("copy", true), Mode::Copy);
        assert_eq!(Mode::choose("Copy", true), Mode::Copy);
        assert_eq!(Mode::choose("iterm", false), Mode::Copy);
        assert_eq!(Mode::choose("", false), Mode::Copy);
    }

    #[test]
    fn a_new_config_resumes_the_way_this_os_can() {
        let cfg = Config::parse(&crate::config::default_toml()).unwrap();
        assert_eq!(cfg.board.resume, platform::default_resume());
        assert_eq!(Config::default().board.resume, platform::default_resume());
        let want = if cfg!(target_os = "macos") {
            Mode::Iterm
        } else {
            Mode::Copy
        };
        assert_eq!(Mode::from_config(&cfg), want);
        // whatever an old or copied config says, Linux copies
        let iterm = Config::parse("[board]\nresume = \"iterm\"").unwrap();
        assert_eq!(Mode::from_config(&iterm), want);
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
            Mode::Iterm,
        );
        let typed = unescape(&p.applescript.unwrap());
        let want: String = p.command.chars().filter(|c| !c.is_control()).collect();
        assert_eq!(typed, want);
        // the shell sees the cwd as one word
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", crate::util::shell_quote(nasty)))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), nasty);
        // a hostile session id is quoted, not run
        let p = plan_command(line("/x", "x; touch /tmp/pwn"), Mode::Copy);
        assert!(p.command.ends_with("--resume 'x; touch /tmp/pwn'"));
    }
}
