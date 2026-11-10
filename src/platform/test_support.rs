//! Programs for tests to put on a search path instead of the real ones.

use super::service::Timing;
use std::fs::{set_permissions, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

/// A program in `dir` that runs `body`.
pub(crate) fn script(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    set_permissions(&p, Permissions::from_mode(0o755)).unwrap(); // NOSONAR: a test stub must run
}

/// A program in `dir` that appends its arguments to `<dir>/<name>.log` and exits with `code`.
pub(crate) fn stub(dir: &Path, name: &str, code: i32) {
    script(
        dir,
        name,
        &format!("echo \"$@\" >> \"$(dirname \"$0\")/{name}.log\"\nexit {code}"),
    );
}

/// Short waits, so a test that has to wait for a stand-in service manager does not take long.
pub(crate) const FAST: Timing = Timing {
    settle: Duration::from_millis(1000),
    poll: Duration::from_millis(5),
    retries: [Duration::from_millis(5); 3],
};

/// A `launchctl` in `dir` that behaves like launchd, and writes each call to `launchctl.log`.
/// Its state is files in `dir`: `loaded` (the service is loaded). `bootout` answers at once and
/// the service goes away only after `print` was asked a few times, and a `bootstrap` that comes
/// before that fails with 5 as the real one does. Settings are files: `unloading` (how many times
/// `print` still shows the service after a bootout; default 2), `bootstrap_fails` (a number: how
/// many bootstraps fail with 5 first), `kickstart_fails` (kickstart fails with 1), `ghost` (a
/// bootstrap that loads nothing).
pub(crate) fn launchd(dir: &Path) {
    script(
        dir,
        "launchctl",
        r#"d="$(dirname "$0")"
echo "$@" >> "$d/launchctl.log"
left() { cat "$d/$1" 2>/dev/null || echo 0; }
case "$1" in
print)
  if [ -f "$d/going" ]; then
    n=$(left going)
    if [ "$n" -gt 0 ]; then echo $((n-1)) > "$d/going"; printf 'pid = 4242\n'; exit 0; fi
    rm -f "$d/going" "$d/loaded"
  fi
  if [ -f "$d/loaded" ]; then printf 'gui/501/dev.agent-eye.daemon = {\n\tstate = running\n\tpid = 4242\n}\n'; exit 0; fi
  echo "Could not find service in domain" >&2; exit 113 ;;
bootout)
  [ -f "$d/loaded" ] || { echo "Boot-out failed: 3: No such process" >&2; exit 3; }
  if [ -f "$d/unloading" ]; then left unloading > "$d/going"; else echo 2 > "$d/going"; fi
  exit 0 ;;
bootstrap)
  n=$(left bootstrap_fails)
  if [ -f "$d/going" ] || [ -f "$d/loaded" ] || [ "$n" -gt 0 ]; then
    [ "$n" -gt 0 ] && echo $((n-1)) > "$d/bootstrap_fails"
    echo "Bootstrap failed: 5: Input/output error" >&2
    echo "Try re-running the command as root for richer errors." >&2
    exit 5
  fi
  [ -f "$d/ghost" ] || touch "$d/loaded"
  exit 0 ;;
kickstart)
  [ -f "$d/kickstart_fails" ] && { echo "Could not kickstart" >&2; exit 1; }
  [ -f "$d/loaded" ] || exit 113
  exit 0 ;;
esac
exit 64"#,
    );
}

/// A `systemctl` in `dir` that behaves like the user's systemd, and writes each call to
/// `systemctl.log`. The unit is active when the file `active` is there. Settings are files:
/// `no_session` (everything fails as it does without a user session), `unknown` (the unit file is
/// not known: `disable` fails).
pub(crate) fn systemd(dir: &Path) {
    script(
        dir,
        "systemctl",
        r#"d="$(dirname "$0")"
echo "$@" >> "$d/systemctl.log"
if [ -f "$d/no_session" ]; then echo "Failed to connect to bus: No medium found" >&2; exit 1; fi
shift
case "$1" in
is-active)
  if [ -f "$d/active" ]; then echo active; exit 0; fi
  echo inactive; exit 3 ;;
daemon-reload) exit 0 ;;
enable|restart) touch "$d/active"; exit 0 ;;
disable)
  if [ -f "$d/unknown" ]; then echo "Failed to disable unit: Unit file agent-eye.service does not exist." >&2; exit 1; fi
  rm -f "$d/active"; exit 0 ;;
esac
exit 64"#,
    );
}

/// What a stand-in program in `dir` was asked, one entry per call: the first word after the
/// options for `systemctl` (`--user` is left out) and the first word for the others.
pub(crate) fn calls(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(format!("{name}.log")))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.split_whitespace()
                .find(|w| !w.starts_with("--"))
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

/// Set a state or a setting of a stand-in (`launchd`, `systemd`): a file with `text` in it.
pub(crate) fn set(dir: &Path, key: &str, text: &str) {
    std::fs::write(dir.join(key), text).unwrap();
}

/// Is the state or setting there?
pub(crate) fn has(dir: &Path, key: &str) -> bool {
    dir.join(key).exists()
}
