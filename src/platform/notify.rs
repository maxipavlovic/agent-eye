//! A local desktop notice: `osascript` on macOS, `notify-send` on Linux when it is installed.
//! The title and the text are always handed over as arguments, never written into a script.

use super::{search_path, MACOS};
use crate::util::find_in;
use std::ffi::OsStr;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a notice program gets before it is killed: a blocked dialog or a missing session
/// bus must not stall the daemon.
const LIMIT: Duration = Duration::from_secs(5);

/// Show a notice. False when nothing could show it: no program, or it failed (a Linux daemon
/// without a desktop session looks like that).
pub fn send(title: &str, body: &str) -> bool {
    send_with(&search_path(), MACOS, title, body, LIMIT)
}

/// Does this machine have a program that can show a notice?
pub fn available() -> bool {
    available_with(&search_path(), MACOS)
}

fn available_with(path: &OsStr, mac: bool) -> bool {
    find_in(command(mac, "", "").0, path).is_some()
}

fn send_with(path: &OsStr, mac: bool, title: &str, body: &str, limit: Duration) -> bool {
    let (program, args) = command(mac, title, body);
    find_in(program, path).is_some_and(|exe| {
        Command::new(exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok_and(|child| succeeds_within(child, limit))
    })
}

/// Did the program exit with success within `limit`? One that is still running then is killed.
fn succeeds_within(mut child: Child, limit: Duration) -> bool {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The program and its arguments. AppleScript's `on run argv` receives what follows `--` as a
/// list of strings, so a quote or a newline in the text is only text.
fn command(mac: bool, title: &str, body: &str) -> (&'static str, Vec<String>) {
    let (program, fixed): (_, &[&str]) = if mac {
        (
            "osascript",
            &[
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
                "--",
            ],
        )
    } else {
        ("notify-send", &["--app-name=agent-eye", "--"])
    };
    let mut args: Vec<String> = fixed.iter().map(|a| a.to_string()).collect();
    args.extend([title.to_string(), body.to_string()]);
    (program, args)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::script;
    use super::*;

    /// A program that writes each argument on a line of `<dir>/<name>.args`.
    fn recorder(dir: &std::path::Path, name: &str, code: i32) {
        script(
            dir,
            name,
            &format!("printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/{name}.args\"\nexit {code}"),
        );
    }

    fn args_of(dir: &std::path::Path, name: &str) -> Vec<String> {
        std::fs::read_to_string(dir.join(format!("{name}.args")))
            .unwrap()
            .lines()
            .map(String::from)
            .collect()
    }

    #[test]
    fn the_text_reaches_osascript_as_arguments_after_the_script() {
        let t = tempfile::tempdir().unwrap();
        recorder(t.path(), "osascript", 0);
        let evil = r#"Ana's reply"; do shell script "touch /tmp/x"; "#;
        assert!(send_with(t.path().as_os_str(), true, "-title", evil, LIMIT));
        let args = args_of(t.path(), "osascript");
        assert_eq!(args[args.len() - 3..], ["--", "-title", evil]);
        let script: String = args[..args.len() - 3].join(" ");
        assert!(
            !script.contains("touch") && !script.contains("-title"),
            "{script}"
        );
        assert!(script.contains("on run argv") && script.contains("end run"));
    }

    #[test]
    fn notify_send_gets_the_text_after_a_double_dash() {
        let t = tempfile::tempdir().unwrap();
        recorder(t.path(), "notify-send", 0);
        assert!(send_with(
            t.path().as_os_str(),
            false,
            "-t",
            "two words",
            LIMIT
        ));
        assert_eq!(
            args_of(t.path(), "notify-send"),
            ["--app-name=agent-eye", "--", "-t", "two words"]
        );
    }

    #[test]
    fn a_notice_is_available_when_the_machines_program_is_on_the_path() {
        let t = tempfile::tempdir().unwrap();
        assert!(!available_with(t.path().as_os_str(), true));
        assert!(!available_with(t.path().as_os_str(), false));
        recorder(t.path(), "osascript", 0);
        assert!(available_with(t.path().as_os_str(), true));
        assert!(!available_with(t.path().as_os_str(), false));
        recorder(t.path(), "notify-send", 0);
        assert!(available_with(t.path().as_os_str(), false));
    }

    #[test]
    fn a_missing_or_failing_program_means_no_notice() {
        let t = tempfile::tempdir().unwrap();
        assert!(!send_with(t.path().as_os_str(), false, "a", "b", LIMIT));
        recorder(t.path(), "notify-send", 1);
        assert!(!send_with(t.path().as_os_str(), false, "a", "b", LIMIT));
    }

    #[test]
    fn a_program_that_hangs_is_killed_after_the_limit() {
        let t = tempfile::tempdir().unwrap();
        script(t.path(), "notify-send", "exec sleep 30");
        let start = Instant::now();
        let sent = send_with(
            t.path().as_os_str(),
            false,
            "a",
            "b",
            Duration::from_millis(200),
        );
        assert!(!sent);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
