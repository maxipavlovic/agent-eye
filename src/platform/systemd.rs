//! Linux: a systemd user unit (`~/.config/systemd/user/agent-eye.service`) run by the user's
//! systemd. The daemon's output goes to the journal (`journalctl --user -u agent-eye`).

use super::service::{How, Runner, Started, Step, Stopped, Unit};
use std::path::{Path, PathBuf};

/// The name of the unit.
pub const SERVICE: &str = "agent-eye.service";

pub fn dir(home: &Path) -> PathBuf {
    home.join(".config/systemd/user")
}

/// Text in double quotes for a unit file: `\`, `"` and `%` (a specifier) are escaped, and a line
/// break cannot start a new setting. `$` is left alone: it only means something in the arguments
/// of `ExecStart`, and in the program's own path it is taken as it is.
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The program of the `ExecStart` line: as it is when its path is plain, else quoted.
fn program(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:=@+,-".contains(&b));
    if plain {
        s.to_string()
    } else {
        quoted(s)
    }
}

/// A `KEY=value` setting of the unit's environment; the quotes go around the whole assignment.
fn env_entry(key: &str, value: &str) -> String {
    format!("Environment={}", quoted(&format!("{key}={value}")))
}

/// The unit file.
pub fn text(u: &Unit) -> String {
    let env: String = u
        .env
        .iter()
        .map(|(k, v)| format!("{}\n", env_entry(k, v)))
        .collect();
    format!(
        "[Unit]
Description=agent-eye daemon: keeps the session index and the board up to date

[Service]
Type=simple
ExecStart={bin} daemon
Restart=on-failure
RestartSec=10
# a terminal opened for Resume lives in this unit's group; a restart must not close it
KillMode=process
{env}
[Install]
WantedBy=default.target
",
        bin = program(&u.bin.to_string_lossy()),
    )
}

/// Does the unit name a home (`EYE_HOME`), and is it this one?
pub fn names_home(text: &str, eye_home: &Path) -> (bool, bool) {
    let ours = env_entry("EYE_HOME", &eye_home.to_string_lossy());
    (
        text.lines()
            .any(|l| l.starts_with("Environment=\"EYE_HOME=")),
        text.lines().any(|l| l == ours),
    )
}

fn systemctl(args: &[&str]) -> Step {
    let mut all = vec!["--user"];
    all.extend(args);
    Step::new("systemctl", &all)
}

fn daemon_reload() -> Step {
    systemctl(&["daemon-reload"])
}

fn enable_now() -> Step {
    systemctl(&["enable", "--now", SERVICE])
}

fn restart() -> Step {
    systemctl(&["restart", SERVICE])
}

fn disable_now() -> Step {
    systemctl(&["disable", "--now", SERVICE])
}

pub fn status() -> Step {
    systemctl(&["is-active", SERVICE])
}

pub fn start(run: &Runner, reload: bool) -> Result<Started, String> {
    let active = run.run(&status())?.ok;
    if active && !reload {
        run.must(&restart())?;
        return confirm(run, How::Restarted);
    }
    run.must(&daemon_reload())?;
    run.must(&enable_now())?;
    if active {
        // enable --now leaves a daemon that is running as it is; it must run the new unit
        run.must(&restart())?;
    }
    confirm(run, if active { How::Reloaded } else { How::Started })
}

/// Is the unit active now?
fn confirm(run: &Runner, how: How) -> Result<Started, String> {
    match run.wait_for(&status(), true)? {
        Some(_) => Ok(Started { how, pid: None }),
        None => Err("systemd does not show the unit as active after starting it".to_string()),
    }
}

pub fn stop(run: &Runner) -> Result<Stopped, String> {
    let active = run.run(&status())?.ok;
    let out = run.run(&disable_now())?;
    if out.ok {
        return Ok(if active {
            Stopped::Stopped
        } else {
            Stopped::NotRunning
        });
    }
    if !active {
        // a unit systemd does not know cannot be disabled, and is not running either
        return Ok(Stopped::NotRunning);
    }
    Err(format!("systemctl disable failed: {}", out.reason()))
}

pub fn forget(run: &Runner) -> Result<(), String> {
    run.must(&daemon_reload()).map(drop)
}

pub fn start_by_hand() -> Vec<Step> {
    vec![daemon_reload(), enable_now()]
}

pub fn stop_by_hand() -> Vec<Step> {
    vec![disable_now()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_for<'a>(bin: &'a Path, env: &'a [(String, String)]) -> Unit<'a> {
        Unit {
            bin,
            logs: Path::new("/home/u/.agent-eye/logs"),
            env,
        }
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_unit_runs_the_daemon_and_restarts_it_when_it_fails() {
        let e = env(&[
            ("PATH", "/home/u/.local/bin:/usr/local/bin:/usr/bin:/bin"),
            ("EYE_CLAUDE_DIR", "/data/claude"),
        ]);
        let text = text(&unit_for(Path::new("/home/u/.local/bin/agent-eye"), &e));
        assert_eq!(
            text,
            "[Unit]
Description=agent-eye daemon: keeps the session index and the board up to date

[Service]
Type=simple
ExecStart=/home/u/.local/bin/agent-eye daemon
Restart=on-failure
RestartSec=10
# a terminal opened for Resume lives in this unit's group; a restart must not close it
KillMode=process
Environment=\"PATH=/home/u/.local/bin:/usr/local/bin:/usr/bin:/bin\"
Environment=\"EYE_CLAUDE_DIR=/data/claude\"

[Install]
WantedBy=default.target
"
        );
    }

    #[test]
    fn odd_paths_are_quoted_and_cannot_add_settings() {
        let e = env(&[
            ("EYE_HOME", "/data/my \"eye\"\nRestart=always"),
            ("X", "50% of $HOME\\"),
        ]);
        let text = text(&unit_for(Path::new("/home/u/my bin/100%/agent-eye"), &e));
        assert!(
            text.contains("ExecStart=\"/home/u/my bin/100%%/agent-eye\" daemon\n"),
            "{text}"
        );
        assert!(
            text.contains("Environment=\"EYE_HOME=/data/my \\\"eye\\\"\\nRestart=always\"\n"),
            "{text}"
        );
        assert!(
            text.contains("Environment=\"X=50%% of $HOME\\\\\"\n"),
            "{text}"
        );
        assert_eq!(text.matches("Restart=").count(), 2, "{text}");
        // systemd takes a `$` in the program's path as it is, and `$$` would stay two
        assert_eq!(program("/p/$HOME/agent-eye"), "\"/p/$HOME/agent-eye\"");
    }

    #[test]
    fn the_unit_names_its_home_only_when_it_is_not_the_default() {
        let none = text(&unit_for(
            Path::new("/b/agent-eye"),
            &env(&[("PATH", "/bin")]),
        ));
        assert_eq!(
            names_home(&none, Path::new("/home/u/.agent-eye")),
            (false, false)
        );
        let e = env(&[("EYE_HOME", "/data/eye")]);
        let some = text(&unit_for(Path::new("/b/agent-eye"), &e));
        assert_eq!(names_home(&some, Path::new("/data/eye")), (true, true));
        assert_eq!(names_home(&some, Path::new("/data/other")), (true, false));
    }

    fn lines(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|s| s.to_string()).collect()
    }

    use crate::platform::test_support::{calls, has, set, systemd as stand_in, FAST};

    /// A stand-in systemd in a folder of its own.
    struct Fake(tempfile::TempDir);

    impl Fake {
        fn new() -> Fake {
            let dir = tempfile::tempdir().unwrap();
            stand_in(dir.path());
            Fake(dir)
        }
        fn run(&self) -> Runner<'_> {
            Runner {
                path: self.0.path().as_os_str(),
                timing: FAST,
            }
        }
        fn calls(&self) -> Vec<String> {
            calls(self.0.path(), "systemctl")
        }
        fn active(&self) -> bool {
            has(self.0.path(), "active")
        }
    }

    #[test]
    fn an_active_unit_with_its_file_unchanged_is_restarted_and_nothing_else() {
        let f = Fake::new();
        set(f.0.path(), "active", "");
        let s = start(&f.run(), false).unwrap();
        assert_eq!(s.how, How::Restarted);
        assert_eq!(f.calls(), ["is-active", "restart", "is-active"]);
    }

    #[test]
    fn a_unit_that_is_not_active_is_reloaded_enabled_and_started() {
        let f = Fake::new();
        let s = start(&f.run(), false).unwrap();
        assert_eq!(s.how, How::Started);
        assert_eq!(
            f.calls(),
            ["is-active", "daemon-reload", "enable", "is-active"]
        );
        assert!(f.active());
    }

    #[test]
    fn an_active_unit_with_a_changed_file_is_reloaded_and_restarted() {
        let f = Fake::new();
        set(f.0.path(), "active", "");
        let s = start(&f.run(), true).unwrap();
        assert_eq!(s.how, How::Reloaded);
        assert_eq!(
            f.calls(),
            [
                "is-active",
                "daemon-reload",
                "enable",
                "restart",
                "is-active"
            ]
        );
    }

    #[test]
    fn a_failing_systemd_is_said_in_a_few_words_and_goes_no_further() {
        let f = Fake::new();
        set(f.0.path(), "no_session", "");
        let e = start(&f.run(), false).unwrap_err();
        assert_eq!(
            e,
            "systemctl daemon-reload failed: Failed to connect to bus: No medium found"
        );
        assert_eq!(f.calls(), ["is-active", "daemon-reload"]);
    }

    #[test]
    fn a_unit_that_is_not_active_after_starting_is_an_error() {
        let t = tempfile::tempdir().unwrap();
        crate::platform::test_support::script(
            t.path(),
            "systemctl",
            "case \"$2\" in is-active) echo failed; exit 3;; esac\nexit 0",
        );
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        let e = start(&run, false).unwrap_err();
        assert!(e.contains("not show the unit as active"), "{e}");
    }

    #[test]
    fn a_missing_systemctl_is_said() {
        let t = tempfile::tempdir().unwrap();
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        assert_eq!(start(&run, false).unwrap_err(), "systemctl was not found");
    }

    #[test]
    fn stopping_disables_the_unit_and_says_what_it_found() {
        let f = Fake::new();
        set(f.0.path(), "active", "");
        assert_eq!(stop(&f.run()).unwrap(), Stopped::Stopped);
        assert!(!f.active());
        assert_eq!(stop(&f.run()).unwrap(), Stopped::NotRunning);
        // a unit systemd does not know is not running; one that runs and cannot be stopped is an error
        set(f.0.path(), "unknown", "");
        assert_eq!(stop(&f.run()).unwrap(), Stopped::NotRunning);
        set(f.0.path(), "active", "");
        let e = stop(&f.run()).unwrap_err();
        assert!(e.starts_with("systemctl disable failed: "), "{e}");
    }

    #[test]
    fn forgetting_a_removed_unit_reloads_systemd() {
        let f = Fake::new();
        forget(&f.run()).unwrap();
        assert_eq!(f.calls(), ["daemon-reload"]);
    }

    #[test]
    fn the_commands_for_a_person_are_the_ones_eye_would_run() {
        assert_eq!(
            lines(&start_by_hand()),
            [
                "systemctl --user daemon-reload",
                "systemctl --user enable --now agent-eye.service",
            ]
        );
        assert_eq!(
            lines(&stop_by_hand()),
            ["systemctl --user disable --now agent-eye.service"]
        );
        assert_eq!(
            status().to_string(),
            "systemctl --user is-active agent-eye.service"
        );
    }

    #[test]
    fn the_unit_goes_to_the_users_systemd_folder() {
        assert_eq!(
            dir(Path::new("/home/u")),
            Path::new("/home/u/.config/systemd/user")
        );
    }
}
