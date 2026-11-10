//! The background service that keeps the daemon running: a launch agent on macOS, a systemd user
//! unit on Linux. A `Manager` builds the service file and knows the commands of its service manager;
//! it runs them only through a `Runner`, so `install` decides when a command may run. Every command
//! runs with its output captured: nothing a service manager prints reaches the terminal.

use super::{launchd, systemd, MACOS};
use crate::util::{find_in, shell_word};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Which service manager runs the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Launchd,
    Systemd,
}

/// What goes into the service file.
pub struct Unit<'a> {
    /// The installed binary; the service runs `<bin> daemon`.
    pub bin: &'a Path,
    /// eye's log folder (launchd writes the daemon's output there; systemd keeps it in the journal).
    pub logs: &'a Path,
    /// The environment the daemon runs with.
    pub env: &'a [(String, String)],
}

/// A service of an older install: its file is removed, and the service is stopped first when it
/// is loaded.
pub struct OldUnit {
    pub file: String,
    /// What the service manager knows it by (a launchd label).
    pub name: String,
}

/// One command of the service manager.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub program: &'static str,
    pub args: Vec<String>,
}

/// What a command printed and how it ended, kept: none of it is shown as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// Why it failed, in a few words: the first thing it said on standard error, without the
    /// program's own prefix (`Bootstrap failed: 5: Input/output error` gives `Input/output
    /// error`), else its exit status.
    pub fn reason(&self) -> String {
        let said = self.stderr.lines().map(str::trim).find(|l| !l.is_empty());
        match (said, self.code) {
            (Some(l), _) => plain_reason(l).to_string(),
            (None, Some(c)) => format!("exit status {c}"),
            (None, None) => "stopped by a signal".to_string(),
        }
    }
}

/// `Bootstrap failed: 5: Input/output error` is `Input/output error`; anything else stays as said.
fn plain_reason(line: &str) -> &str {
    let tail = line
        .split_once(" failed: ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .filter(|(code, _)| !code.is_empty() && code.bytes().all(|b| b.is_ascii_digit()));
    tail.map_or(line, |(_, text)| text)
}

impl Step {
    pub(super) fn new(program: &'static str, args: &[&str]) -> Step {
        Step {
            program,
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// Run it, looking the program up in `path`, and keep what it printed. None when the program
    /// is not installed. It reads nothing from the terminal and writes nothing to it.
    pub fn capture(&self, path: &OsStr) -> Option<Captured> {
        let exe = find_in(self.program, path)?;
        // Linux refuses to run a file another thread still has open for writing ("text file
        // busy"), which happens when a script was written a moment ago; it passes on its own.
        let mut tries = 0;
        let out = loop {
            match Command::new(&exe).args(&self.args).output() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                r => break r.ok()?,
            }
        };
        Some(Captured {
            ok: out.status.success(),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Run it for what it prints: whether it succeeded and its standard output, trimmed. None when
    /// the program is not installed.
    pub fn read(&self, path: &OsStr) -> Option<(bool, String)> {
        let out = self.capture(path)?;
        Some((out.ok, out.stdout.trim().to_string()))
    }

    /// The program and what it is asked to do, without options and targets (`launchctl bootstrap`),
    /// for a message.
    pub fn short(&self) -> String {
        match self.args.iter().find(|a| !a.starts_with('-')) {
            Some(verb) => format!("{} {verb}", self.program),
            None => self.program.to_string(),
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for a in &self.args {
            write!(f, " {}", shell_word(a))?;
        }
        Ok(())
    }
}

/// How long to wait for a service manager, which does its work after it has answered.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How long a service gets to go away after it was told to, and to show up after it was started.
    pub settle: Duration,
    /// How often to look while waiting.
    pub poll: Duration,
    /// The pauses before each retry of a start that failed because the service was still going away.
    pub retries: [Duration; 3],
}

impl Timing {
    pub const REAL: Timing = Timing {
        settle: Duration::from_secs(5),
        poll: Duration::from_millis(200),
        retries: [
            Duration::from_millis(300),
            Duration::from_millis(600),
            Duration::from_millis(1200),
        ],
    };
}

/// Runs the service manager's commands from a search path, waiting where the manager answers
/// before it is done.
pub struct Runner<'a> {
    pub path: &'a OsStr,
    pub timing: Timing,
}

impl Runner<'_> {
    /// Run a step; its program not being there is an error in our words.
    pub(super) fn run(&self, s: &Step) -> Result<Captured, String> {
        s.capture(self.path)
            .ok_or_else(|| format!("{} was not found", s.program))
    }

    /// Run a step that has to work; its reason says what failed.
    pub(super) fn must(&self, s: &Step) -> Result<Captured, String> {
        let out = self.run(s)?;
        if out.ok {
            Ok(out)
        } else {
            Err(format!("{} failed: {}", s.short(), out.reason()))
        }
    }

    /// Look at `s` until it succeeds (`present`) or stops succeeding (`!present`), for up to the
    /// settle time. What the last look gave, when it is what was waited for.
    pub(super) fn wait_for(&self, s: &Step, present: bool) -> Result<Option<Captured>, String> {
        let t = self.timing;
        let until = Instant::now() + t.settle;
        loop {
            let out = self.run(s)?;
            if out.ok == present {
                return Ok(Some(out));
            }
            if Instant::now() >= until {
                return Ok(None);
            }
            std::thread::sleep(t.poll);
        }
    }

    /// Run `s` again after each pause of the timing's retries for as long as `again` says its
    /// answer is worth another try.
    pub(super) fn retrying(
        &self,
        s: &Step,
        again: impl Fn(&Captured) -> bool,
    ) -> Result<Captured, String> {
        let mut pauses = self.timing.retries.iter();
        loop {
            let out = self.run(s)?;
            match pauses.next() {
                Some(pause) if !out.ok && again(&out) => std::thread::sleep(*pause),
                _ => return Ok(out),
            }
        }
    }
}

/// What a start did to the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum How {
    /// It was running on the old binary and runs on the new one now.
    Restarted,
    /// It was running from an older service file and was loaded again from the new one.
    Reloaded,
    /// It was not loaded and now is.
    Started,
}

/// A service that is up, with its process id when the manager tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    pub how: How,
    pub pid: Option<u32>,
}

/// What a stop found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Stopped,
    /// It was not loaded or not running.
    NotRunning,
}

impl Manager {
    /// The manager of the OS eye runs on.
    pub fn current() -> Manager {
        if MACOS {
            Manager::Launchd
        } else {
            Manager::Systemd
        }
    }

    /// What the service file is called in messages.
    pub fn what(self) -> &'static str {
        match self {
            Manager::Launchd => "launch agent",
            Manager::Systemd => "systemd unit",
        }
    }

    /// Where the service file goes by default, for a user whose home folder is `home`.
    pub fn default_dir(self, home: &Path) -> PathBuf {
        match self {
            Manager::Launchd => launchd::dir(home),
            Manager::Systemd => systemd::dir(home),
        }
    }

    /// The name of the service file.
    pub fn file_name(self) -> String {
        match self {
            Manager::Launchd => launchd::file_name(launchd::LABEL),
            Manager::Systemd => systemd::SERVICE.to_string(),
        }
    }

    /// The text of the service file.
    pub fn unit_text(self, u: &Unit) -> String {
        match self {
            Manager::Launchd => launchd::text(u),
            Manager::Systemd => systemd::text(u),
        }
    }

    /// Does this service file run the daemon of `eye_home`? A file names a home when it is not
    /// `~/.agent-eye` and names none otherwise; `default_home` is the user whose `~/.agent-eye`
    /// that would be.
    pub fn serves_home(self, text: &str, eye_home: &Path, default_home: Option<&Path>) -> bool {
        let (names_one, names_this) = match self {
            Manager::Launchd => launchd::names_home(text, eye_home),
            Manager::Systemd => systemd::names_home(text, eye_home),
        };
        if names_one {
            names_this
        } else {
            default_home.is_some_and(|h| h.join(".agent-eye") == eye_home)
        }
    }

    /// Service files an older eye wrote, which install and uninstall clean up.
    pub fn old_units(self) -> Vec<OldUnit> {
        match self {
            Manager::Launchd => launchd::old_units(),
            Manager::Systemd => vec![],
        }
    }

    /// What the service manager knows the service by: the launchd label, the systemd unit.
    pub fn service(self) -> &'static str {
        match self {
            Manager::Launchd => launchd::LABEL,
            Manager::Systemd => systemd::SERVICE,
        }
    }

    /// Get the daemon running from the service file at `file`, and never leave it down while doing
    /// so. Running and `reload` not asked for (the file is as it was): it is restarted in place, so
    /// it runs the binary now installed. Running and `reload`: it is unloaded, waited for, and
    /// loaded again. Not running: it is started. The answer is the service's state, asked for after.
    pub fn start(self, run: &Runner, file: &Path, reload: bool) -> Result<Started, String> {
        match self {
            Manager::Launchd => launchd::start(run, file, reload),
            Manager::Systemd => systemd::start(run, reload),
        }
    }

    /// Stop the service of an older install, when it is loaded; true when it was.
    pub fn retire(self, run: &Runner, old: &OldUnit) -> Result<bool, String> {
        match self {
            Manager::Launchd => launchd::retire(run, &old.name),
            Manager::Systemd => Ok(false),
        }
    }

    /// Stop the daemon and unload the service, before its file is removed.
    pub fn stop(self, run: &Runner) -> Result<Stopped, String> {
        match self {
            Manager::Launchd => launchd::stop(run),
            Manager::Systemd => systemd::stop(run),
        }
    }

    /// Tell the manager the service file is gone, after it was removed.
    pub fn forget(self, run: &Runner) -> Result<(), String> {
        match self {
            Manager::Launchd => Ok(()),
            Manager::Systemd => systemd::forget(run),
        }
    }

    /// The commands a person runs to start the daemon from the service file at `file`, for when
    /// eye may not run them.
    pub fn start_by_hand(self, file: &Path) -> Vec<Step> {
        match self {
            Manager::Launchd => launchd::start_by_hand(file),
            Manager::Systemd => systemd::start_by_hand(),
        }
    }

    /// The commands a person runs to stop the daemon and unload the service.
    pub fn stop_by_hand(self) -> Vec<Step> {
        match self {
            Manager::Launchd => launchd::stop_by_hand(),
            Manager::Systemd => systemd::stop_by_hand(),
        }
    }

    /// The command that says whether the service is running, for `doctor`, if there is one.
    pub fn status(self) -> Option<Step> {
        match self {
            Manager::Launchd => None,
            Manager::Systemd => Some(systemd::status()),
        }
    }

    /// How to start the daemon from the service manager, for a message.
    pub fn start_hint(self) -> &'static str {
        match self {
            Manager::Launchd => "launchctl bootstrap",
            Manager::Systemd => "systemctl --user start agent-eye",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_support::{script, stub, FAST};

    #[test]
    fn the_manager_follows_the_os() {
        assert_eq!(
            Manager::current(),
            if cfg!(target_os = "macos") {
                Manager::Launchd
            } else {
                Manager::Systemd
            }
        );
    }

    #[test]
    fn a_step_prints_as_the_command_line() {
        let s = Step::new("systemctl", &["--user", "restart", "agent-eye.service"]);
        assert_eq!(s.to_string(), "systemctl --user restart agent-eye.service");
        let s = Step::new("launchctl", &["bootstrap", "gui/501", "/Users/a b/x.plist"]);
        assert_eq!(
            s.to_string(),
            "launchctl bootstrap gui/501 '/Users/a b/x.plist'"
        );
    }

    #[test]
    fn a_step_runs_the_program_found_on_the_given_path_and_keeps_what_it_said() {
        let t = tempfile::tempdir().unwrap();
        script(
            t.path(),
            "launchctl",
            "echo out\necho 'Bootstrap failed: 5: Input/output error' >&2\necho 'Try re-running the command as root for richer errors.' >&2\nexit 5",
        );
        let s = Step::new("launchctl", &["bootstrap", "gui/501", "/x.plist"]);
        let got = s.capture(t.path().as_os_str()).unwrap();
        assert!(!got.ok && got.code == Some(5));
        assert_eq!(got.stdout, "out\n");
        assert!(
            got.stderr
                .starts_with("Bootstrap failed: 5: Input/output error\n"),
            "kept, not shown: {}",
            got.stderr
        );
        assert_eq!(got.reason(), "Input/output error");
        stub(t.path(), "systemctl", 0);
        let ok = Step::new("systemctl", &["--user", "daemon-reload"]);
        assert!(ok.capture(t.path().as_os_str()).unwrap().ok);
        let log = std::fs::read_to_string(t.path().join("systemctl.log")).unwrap();
        assert_eq!(log.trim(), "--user daemon-reload");
    }

    #[test]
    fn a_failure_is_said_in_the_commands_own_few_words() {
        let say = |stderr: &str, code| {
            Captured {
                ok: false,
                code,
                stdout: String::new(),
                stderr: stderr.to_string(),
            }
            .reason()
        };
        assert_eq!(
            say(
                "Bootstrap failed: 5: Input/output error\nTry re-running",
                Some(5)
            ),
            "Input/output error"
        );
        assert_eq!(
            say("\nFailed to connect to bus: No medium found\n", Some(1)),
            "Failed to connect to bus: No medium found"
        );
        assert_eq!(
            say("Operation failed: no code here", Some(1)),
            "Operation failed: no code here"
        );
        assert_eq!(say("", Some(3)), "exit status 3");
        assert_eq!(say("  \n", None), "stopped by a signal");
    }

    #[test]
    fn a_step_is_named_by_its_program_and_what_it_does() {
        let s = Step::new(
            "systemctl",
            &["--user", "enable", "--now", "agent-eye.service"],
        );
        assert_eq!(s.short(), "systemctl enable");
        let s = Step::new("launchctl", &["bootstrap", "gui/501", "/x.plist"]);
        assert_eq!(s.short(), "launchctl bootstrap");
    }

    #[test]
    fn a_missing_program_is_not_found() {
        let t = tempfile::tempdir().unwrap();
        let s = Step::new("systemctl", &["--user", "daemon-reload"]);
        assert!(s.capture(t.path().as_os_str()).is_none());
        assert!(s.read(t.path().as_os_str()).is_none());
    }

    #[test]
    fn read_gives_the_output_and_whether_it_worked() {
        let t = tempfile::tempdir().unwrap();
        script(t.path(), "systemctl", "echo inactive\nexit 3");
        let s = Step::new("systemctl", &["--user", "is-active", "agent-eye.service"]);
        assert_eq!(
            s.read(t.path().as_os_str()),
            Some((false, "inactive".to_string()))
        );
    }

    #[test]
    fn waiting_looks_until_the_answer_changes_and_gives_up_after_the_settle_time() {
        let t = tempfile::tempdir().unwrap();
        script(
            t.path(),
            "launchctl",
            "d=\"$(dirname \"$0\")\"\necho x >> \"$d/n\"\n[ \"$(wc -l < \"$d/n\")\" -ge 4 ]",
        );
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        let step = Step::new("launchctl", &["print", "x"]);
        assert!(run.wait_for(&step, true).unwrap().is_some());
        let looks = std::fs::read_to_string(t.path().join("n"))
            .unwrap()
            .lines()
            .count();
        assert_eq!(looks, 4, "it looked until it worked");
        // never gone: it gives up
        assert!(run.wait_for(&step, false).unwrap().is_none());
    }

    #[test]
    fn a_retry_waits_and_stops_when_the_answer_is_not_worth_another_try() {
        let t = tempfile::tempdir().unwrap();
        stub(t.path(), "launchctl", 5);
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        let step = Step::new("launchctl", &["bootstrap", "x"]);
        let out = run.retrying(&step, |o| o.code == Some(5)).unwrap();
        assert!(!out.ok);
        let log = std::fs::read_to_string(t.path().join("launchctl.log")).unwrap();
        assert_eq!(log.lines().count(), 4);
        std::fs::remove_file(t.path().join("launchctl.log")).unwrap();
        run.retrying(&step, |_| false).unwrap();
        let log = std::fs::read_to_string(t.path().join("launchctl.log")).unwrap();
        assert_eq!(log.lines().count(), 1);
    }
}
