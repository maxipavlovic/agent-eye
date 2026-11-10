//! macOS: a launch agent (`~/Library/LaunchAgents/dev.agent-eye.daemon.plist`) run by launchd.
//!
//! `launchctl bootout` answers before the service is gone, and a `bootstrap` that comes while it is
//! still going away fails with 5 (Input/output error). So a service is only ever unloaded when its
//! file changed, eye waits until `launchctl print` no longer shows it, and a restart for a new
//! binary is a `kickstart -k`, which never unloads anything.

use super::service::{How, OldUnit, Runner, Started, Step, Stopped, Unit};
use super::uid;
use std::path::{Path, PathBuf};

/// The launchd label of the daemon.
pub const LABEL: &str = "dev.agent-eye.daemon";
/// The label used before the rename; install and uninstall clean it up.
pub const OLD_LABEL: &str = "dev.claude-eye.daemon";

pub fn dir(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
}

pub fn file_name(label: &str) -> String {
    format!("{label}.plist")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn env_entry(key: &str, value: &str) -> String {
    format!("<key>{}</key><string>{}</string>", xml(key), xml(value))
}

/// The launch agent's plist.
pub fn text(u: &Unit) -> String {
    let env: String = u
        .env
        .iter()
        .map(|(k, v)| format!("    {}\n", env_entry(k, v)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{bin}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>{out}</string>
  <key>StandardErrorPath</key><string>{err}</string>
  <key>EnvironmentVariables</key>
  <dict>
{env}  </dict>
</dict>
</plist>
"#,
        label = LABEL,
        bin = xml(&u.bin.to_string_lossy()),
        out = xml(&u.logs.join("launchd.out.log").to_string_lossy()),
        err = xml(&u.logs.join("launchd.err.log").to_string_lossy()),
    )
}

/// Does the plist name a home (`EYE_HOME`), and is it this one?
pub fn names_home(text: &str, eye_home: &Path) -> (bool, bool) {
    let ours = env_entry("EYE_HOME", &eye_home.to_string_lossy());
    (text.contains("<key>EYE_HOME</key>"), text.contains(&ours))
}

fn target(label: &str) -> String {
    format!("gui/{}/{label}", uid())
}

fn print(label: &str) -> Step {
    Step::new("launchctl", &["print", &target(label)])
}

fn bootout(label: &str) -> Step {
    Step::new("launchctl", &["bootout", &target(label)])
}

fn bootstrap(file: &Path) -> Step {
    Step::new(
        "launchctl",
        &[
            "bootstrap",
            &format!("gui/{}", uid()),
            &file.to_string_lossy(),
        ],
    )
}

fn kickstart(label: &str) -> Step {
    Step::new("launchctl", &["kickstart", "-k", &target(label)])
}

pub fn old_units() -> Vec<OldUnit> {
    vec![OldUnit {
        file: file_name(OLD_LABEL),
        name: OLD_LABEL.to_string(),
    }]
}

/// The process id in what `launchctl print` says about a running service (`pid = 1234`).
fn pid_in(printed: &str) -> Option<u32> {
    printed
        .lines()
        .find_map(|l| l.trim().strip_prefix("pid = "))
        .and_then(|n| n.trim().parse().ok())
}

pub fn start(run: &Runner, file: &Path, reload: bool) -> Result<Started, String> {
    let loaded = run.run(&print(LABEL))?.ok;
    if loaded && !reload && run.run(&kickstart(LABEL))?.ok {
        return confirm(run, How::Restarted);
    }
    // loaded under an older file (or a restart that did not work): unload it, wait until it is
    // gone, load it again
    if loaded {
        unload(run, LABEL)?;
    }
    let out = run.retrying(&bootstrap(file), |o| o.code == Some(5))?;
    if !out.ok {
        return Err(format!("launchctl bootstrap failed: {}", out.reason()));
    }
    confirm(run, if loaded { How::Reloaded } else { How::Started })
}

/// Ask launchd to unload the service and wait until it is gone. Not being loaded is fine; a
/// service that stays is not.
fn unload(run: &Runner, label: &str) -> Result<(), String> {
    run.run(&bootout(label))?;
    match run.wait_for(&print(label), false)? {
        Some(_) => Ok(()),
        None => Err("launchctl bootout did not finish: the service is still loaded".to_string()),
    }
}

/// Is the service loaded now? It can take a moment to show after a bootstrap or a kickstart.
fn confirm(run: &Runner, how: How) -> Result<Started, String> {
    match run.wait_for(&print(LABEL), true)? {
        Some(printed) => Ok(Started {
            how,
            pid: pid_in(&printed.stdout),
        }),
        None => Err("launchd does not show the service after starting it".to_string()),
    }
}

pub fn retire(run: &Runner, label: &str) -> Result<bool, String> {
    let loaded = run.run(&print(label))?.ok;
    if loaded {
        unload(run, label)?;
    }
    Ok(loaded)
}

pub fn stop(run: &Runner) -> Result<Stopped, String> {
    if !run.run(&print(LABEL))?.ok {
        return Ok(Stopped::NotRunning);
    }
    unload(run, LABEL)?;
    Ok(Stopped::Stopped)
}

pub fn start_by_hand(file: &Path) -> Vec<Step> {
    vec![bootstrap(file)]
}

pub fn stop_by_hand() -> Vec<Step> {
    vec![bootout(LABEL)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(steps: &[Step]) -> Vec<String> {
        steps.iter().map(|s| s.to_string()).collect()
    }

    /// What the launch agent has been since the first release: it must not change.
    const GOLDEN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.agent-eye.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/u/.local/bin/agent-eye</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/u/work/eye/logs/launchd.out.log</string>
  <key>StandardErrorPath</key><string>/Users/u/work/eye/logs/launchd.err.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>/Users/u/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
    <key>EYE_HOME</key><string>/Users/u/work/eye</string>
    <key>EYE_CLAUDE_DIR</key><string>/Users/u/claude dir</string>
  </dict>
</dict>
</plist>
"#;

    #[test]
    fn the_plist_is_what_it_has_always_been() {
        let env = [
            (
                "PATH".to_string(),
                "/Users/u/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
            ),
            ("EYE_HOME".to_string(), "/Users/u/work/eye".to_string()),
            (
                "EYE_CLAUDE_DIR".to_string(),
                "/Users/u/claude dir".to_string(),
            ),
        ];
        let text = text(&Unit {
            bin: Path::new("/Users/u/.local/bin/agent-eye"),
            logs: Path::new("/Users/u/work/eye/logs"),
            env: &env,
        });
        assert_eq!(text, GOLDEN);
    }

    #[test]
    fn the_label_and_the_plist_path_stay_as_installed() {
        assert_eq!(LABEL, "dev.agent-eye.daemon");
        assert_eq!(file_name(LABEL), "dev.agent-eye.daemon.plist");
        assert_eq!(
            dir(Path::new("/Users/u")),
            Path::new("/Users/u/Library/LaunchAgents")
        );
        assert_eq!(file_name(OLD_LABEL), "dev.claude-eye.daemon.plist");
    }

    #[test]
    fn plist_values_are_escaped() {
        let env = [("A&B".to_string(), "<x>&".to_string())];
        let text = text(&Unit {
            bin: Path::new("/a&b/agent-eye"),
            logs: Path::new("/l"),
            env: &env,
        });
        assert!(
            text.contains("<string>/a&amp;b/agent-eye</string>"),
            "{text}"
        );
        assert!(
            text.contains("<key>A&amp;B</key><string>&lt;x&gt;&amp;</string>"),
            "{text}"
        );
    }

    #[test]
    fn it_names_its_home_only_when_it_is_not_the_default() {
        let plain = [("PATH".to_string(), "/bin".to_string())];
        let named = [("EYE_HOME".to_string(), "/data/eye".to_string())];
        let unit = |env: &[(String, String)]| {
            text(&Unit {
                bin: Path::new("/b/agent-eye"),
                logs: Path::new("/l"),
                env,
            })
        };
        assert_eq!(names_home(&unit(&plain), Path::new("/x")), (false, false));
        assert_eq!(
            names_home(&unit(&named), Path::new("/data/eye")),
            (true, true)
        );
        assert_eq!(
            names_home(&unit(&named), Path::new("/data/o")),
            (true, false)
        );
    }

    use crate::platform::test_support::{calls, has, launchd as stand_in, set, FAST};

    /// A stand-in launchd in a folder of its own, and the plist a test starts from.
    struct Fake {
        dir: tempfile::TempDir,
        plist: PathBuf,
    }

    impl Fake {
        fn new() -> Fake {
            let dir = tempfile::tempdir().unwrap();
            stand_in(dir.path());
            let plist = dir.path().join("agent-eye.plist");
            Fake { dir, plist }
        }
        fn run(&self) -> Runner<'_> {
            Runner {
                path: self.dir.path().as_os_str(),
                timing: FAST,
            }
        }
        fn calls(&self) -> Vec<String> {
            calls(self.dir.path(), "launchctl")
        }
        fn loaded(&self) -> bool {
            has(self.dir.path(), "loaded")
        }
    }

    #[test]
    fn a_loaded_service_with_its_file_unchanged_is_kickstarted_and_never_unloaded() {
        let f = Fake::new();
        set(f.dir.path(), "loaded", "");
        let s = start(&f.run(), &f.plist, false).unwrap();
        assert_eq!(s.how, How::Restarted);
        assert_eq!(s.pid, Some(4242));
        assert_eq!(f.calls(), ["print", "kickstart", "print"]);
        assert!(f.loaded());
    }

    #[test]
    fn a_service_that_is_not_loaded_is_bootstrapped() {
        let f = Fake::new();
        let s = start(&f.run(), &f.plist, false).unwrap();
        assert_eq!(s.how, How::Started);
        assert_eq!(f.calls(), ["print", "bootstrap", "print"]);
        assert!(f.loaded());
    }

    #[test]
    fn a_changed_file_unloads_waits_until_it_is_gone_and_loads_it_again() {
        let f = Fake::new();
        set(f.dir.path(), "loaded", "");
        // launchd goes on showing the service for a while after bootout has answered
        set(f.dir.path(), "unloading", "4");
        let s = start(&f.run(), &f.plist, true).unwrap();
        assert_eq!(s.how, How::Reloaded);
        let calls = f.calls();
        assert_eq!(calls[..2], ["print", "bootout"]);
        let boots = calls.iter().filter(|c| *c == "bootstrap").count();
        assert_eq!(boots, 1, "no bootstrap came too early: {calls:?}");
        let at = calls.iter().position(|c| c == "bootstrap").unwrap();
        assert!(
            calls[2..at].iter().filter(|c| *c == "print").count() >= 4,
            "it looked until the service was gone: {calls:?}"
        );
        assert!(f.loaded());
    }

    #[test]
    fn a_bootstrap_that_fails_with_5_is_tried_again() {
        let f = Fake::new();
        set(f.dir.path(), "bootstrap_fails", "2");
        let s = start(&f.run(), &f.plist, false).unwrap();
        assert_eq!(s.how, How::Started);
        let n = f.calls().iter().filter(|c| *c == "bootstrap").count();
        assert_eq!(n, 3, "two failures, then it worked");
    }

    #[test]
    fn a_bootstrap_that_never_works_is_said_in_a_few_words() {
        let f = Fake::new();
        set(f.dir.path(), "bootstrap_fails", "99");
        let e = start(&f.run(), &f.plist, false).unwrap_err();
        assert_eq!(e, "launchctl bootstrap failed: Input/output error");
        let n = f.calls().iter().filter(|c| *c == "bootstrap").count();
        assert_eq!(n, 4, "the first try and three more");
    }

    #[test]
    fn other_failures_are_not_retried() {
        let t = tempfile::tempdir().unwrap();
        crate::platform::test_support::script(
            t.path(),
            "launchctl",
            "echo \"$1\" >> \"$(dirname \"$0\")/n\"\ncase \"$1\" in print) exit 113;; esac\necho 'Bootstrap failed: 119: Service is disabled' >&2\nexit 119",
        );
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        let e = start(&run, Path::new("/x.plist"), false).unwrap_err();
        assert_eq!(e, "launchctl bootstrap failed: Service is disabled");
        let log = std::fs::read_to_string(t.path().join("n")).unwrap();
        assert_eq!(log.matches("bootstrap").count(), 1, "{log}");
    }

    #[test]
    fn a_kickstart_that_fails_falls_back_to_unloading_and_loading() {
        let f = Fake::new();
        set(f.dir.path(), "loaded", "");
        set(f.dir.path(), "kickstart_fails", "");
        let s = start(&f.run(), &f.plist, false).unwrap();
        assert_eq!(s.how, How::Reloaded);
        let calls = f.calls();
        assert!(
            calls.contains(&"kickstart".into()) && calls.contains(&"bootout".into()),
            "{calls:?}"
        );
        assert!(f.loaded());
    }

    #[test]
    fn a_service_that_does_not_go_away_is_said() {
        let f = Fake::new();
        set(f.dir.path(), "loaded", "");
        set(f.dir.path(), "unloading", "100000");
        let e = start(&f.run(), &f.plist, true).unwrap_err();
        assert!(e.contains("bootout did not finish"), "{e}");
        assert!(!f.calls().contains(&"bootstrap".into()), "{:?}", f.calls());
    }

    #[test]
    fn a_service_that_is_not_there_after_starting_is_an_error() {
        let f = Fake::new();
        set(f.dir.path(), "ghost", "");
        let e = start(&f.run(), &f.plist, false).unwrap_err();
        assert!(e.contains("does not show the service"), "{e}");
    }

    #[test]
    fn a_missing_launchctl_is_said() {
        let t = tempfile::tempdir().unwrap();
        let run = Runner {
            path: t.path().as_os_str(),
            timing: FAST,
        };
        assert_eq!(
            start(&run, Path::new("/x.plist"), false).unwrap_err(),
            "launchctl was not found"
        );
    }

    #[test]
    fn stopping_waits_until_the_service_is_gone() {
        let f = Fake::new();
        assert_eq!(stop(&f.run()).unwrap(), Stopped::NotRunning);
        assert_eq!(f.calls(), ["print"], "nothing to boot out");
        set(f.dir.path(), "loaded", "");
        set(f.dir.path(), "unloading", "3");
        assert_eq!(stop(&f.run()).unwrap(), Stopped::Stopped);
        assert!(!f.loaded(), "gone when stop returns");
        let calls = f.calls();
        let out = calls.iter().position(|c| c == "bootout").unwrap();
        assert!(
            calls[out..].iter().filter(|c| *c == "print").count() >= 3,
            "{calls:?}"
        );
    }

    #[test]
    fn an_older_service_is_unloaded_only_when_it_is_loaded() {
        let f = Fake::new();
        assert!(!retire(&f.run(), OLD_LABEL).unwrap());
        assert_eq!(f.calls(), ["print"]);
        set(f.dir.path(), "loaded", "");
        assert!(retire(&f.run(), OLD_LABEL).unwrap());
        assert!(!f.loaded());
    }

    #[test]
    fn the_pid_is_read_from_what_print_says() {
        assert_eq!(
            pid_in("x = {\n\tstate = running\n\tpid = 812\n}\n"),
            Some(812)
        );
        assert_eq!(pid_in("x = {\n\tstate = waiting\n}\n"), None);
    }

    #[test]
    fn the_commands_are_launchctls_for_this_user() {
        let id = uid();
        let plist = Path::new("/Users/u/Library/LaunchAgents/dev.agent-eye.daemon.plist");
        assert_eq!(
            lines(&start_by_hand(plist)),
            [format!("launchctl bootstrap gui/{id} {}", plist.display())]
        );
        assert_eq!(
            lines(&stop_by_hand()),
            [format!("launchctl bootout gui/{id}/dev.agent-eye.daemon")]
        );
        assert_eq!(
            kickstart(LABEL).to_string(),
            format!("launchctl kickstart -k gui/{id}/dev.agent-eye.daemon")
        );
        assert_eq!(
            print(LABEL).to_string(),
            format!("launchctl print gui/{id}/dev.agent-eye.daemon")
        );
        let old = old_units();
        assert_eq!(old[0].file, "dev.claude-eye.daemon.plist");
        assert_eq!(old[0].name, "dev.claude-eye.daemon");
    }
}
