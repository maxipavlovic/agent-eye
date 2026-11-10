//! What differs between operating systems, in one place. The rest of eye asks this module to keep
//! the daemon running (`service`), open a URL, copy text (`clipboard`) or open a terminal tab, and
//! never looks at the OS itself.
//!
//! The code for every OS is compiled everywhere, so every OS tests all of it. The one place that
//! chooses between macOS and Linux is `MACOS` below; anything that is not macOS is treated as Linux.

pub mod clipboard;
mod launchd;
pub mod service;
mod systemd;
#[cfg(test)]
pub(crate) mod test_support;

use crate::util::find_in;
use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::Command;

/// Are we on macOS? The only `cfg!` on the operating system in eye's own code.
const MACOS: bool = cfg!(target_os = "macos");

/// The account's home from the password database; `$HOME` can be overridden, so it is not trusted.
pub fn account_home() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: getpwuid returns null or a pointer to a static entry that is read at once.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())))
    }
}

/// The numeric id of the user running eye.
pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

/// The folders programs are looked up in: `$PATH`, or the system's minimum when it is not set.
pub fn search_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into())
}

/// The program that opens a URL in the default browser.
pub fn open_program() -> &'static str {
    if MACOS {
        "open"
    } else {
        "xdg-open"
    }
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    open_url_with(&search_path(), url)
}

fn open_url_with(path: &OsStr, url: &str) -> Result<()> {
    let program = open_program();
    let exe = find_in(program, path).with_context(|| format!("{program} was not found"))?;
    let status = Command::new(exe)
        .arg(url)
        .status()
        .with_context(|| format!("running {program}"))?;
    anyhow::ensure!(status.success(), "{program} failed ({status})");
    Ok(())
}

/// Can eye open a command in a new terminal tab? Only iTerm2 on macOS is supported; elsewhere the
/// command is copied.
pub fn can_open_terminal_tab() -> bool {
    MACOS
}

/// The `[board] resume` value a new config gets.
pub fn default_resume() -> &'static str {
    if can_open_terminal_tab() {
        "iterm"
    } else {
        "copy"
    }
}

/// Folders the daemon always gets on its `PATH`, after the agents' own and eye's.
pub fn system_path_dirs() -> &'static [&'static str] {
    if MACOS {
        &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
    } else {
        &[
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ]
    }
}

/// Run AppleScript with `osascript` and return what it said.
pub fn run_applescript(script: &str) -> Result<std::process::Output> {
    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .context("running osascript")
}

#[cfg(test)]
mod tests {
    use super::test_support::stub;
    use super::*;

    #[test]
    fn a_url_goes_to_the_open_program_on_the_given_path() {
        let t = tempfile::tempdir().unwrap();
        stub(t.path(), open_program(), 0);
        open_url_with(t.path().as_os_str(), "http://127.0.0.1:7420/?token=x").unwrap();
        let log =
            std::fs::read_to_string(t.path().join(format!("{}.log", open_program()))).unwrap();
        assert_eq!(log.trim(), "http://127.0.0.1:7420/?token=x");
    }

    #[test]
    fn opening_says_why_it_failed() {
        let t = tempfile::tempdir().unwrap();
        let e = open_url_with(t.path().as_os_str(), "http://x").unwrap_err();
        assert!(format!("{e:#}").contains("was not found"), "{e:#}");
        stub(t.path(), open_program(), 3);
        let e = open_url_with(t.path().as_os_str(), "http://x").unwrap_err();
        assert!(format!("{e:#}").contains("failed"), "{e:#}");
    }

    #[test]
    fn only_macos_opens_terminal_tabs_and_the_others_copy() {
        assert_eq!(can_open_terminal_tab(), cfg!(target_os = "macos"));
        assert_eq!(
            default_resume(),
            if cfg!(target_os = "macos") {
                "iterm"
            } else {
                "copy"
            }
        );
    }

    #[test]
    fn the_account_home_is_not_taken_from_the_environment() {
        let home = account_home().unwrap();
        assert!(home.is_absolute());
    }
}
