//! The `eye` a night run finds first.
//!
//! The daemon's `PATH` puts the agents' folders first, so `eye` inside a run was whichever copy sits
//! next to an agent, not the one the daemon is (an old package manager's copy next to `claude`
//! was one version behind). Every run therefore starts with `<eye home>/bin` on its `PATH`, a
//! folder that holds only `eye` and `agent-eye`, links to the binary running now.

use crate::install::{ALIAS_NAME, BIN_NAME};
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The folder of links, inside eye's own.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.home.join("bin")
}

/// Make `dir` hold `eye` and `agent-eye`, both links to this binary, which is returned. A link
/// that already points there is left alone, so a run that starts while another reads it never
/// finds it missing.
pub fn refresh(dir: &Path) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding the running eye")?;
    note_running();
    link_to(dir, &exe)?;
    Ok(exe)
}

/// A file as the disk knows it: device and inode, after links.
fn identity(path: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.dev(), m.ino()))
}

/// The file this process was started from, noted once.
static RUNNING: OnceLock<Option<(u64, u64)>> = OnceLock::new();

/// Note which file this process runs, so that a later start can tell when a package manager has
/// put another file at the same path (the links go to the path, not to the file). The daemon notes
/// it as it starts; any other process notes it at its first start of a run.
pub fn note_running() {
    RUNNING.get_or_init(|| identity(&std::env::current_exe().ok()?));
}

/// Is the file at `path` another one than `was`? Nothing known, or nothing there, says no.
fn replaced_since(was: Option<(u64, u64)>, path: &Path) -> bool {
    was.zip(identity(path)).is_some_and(|(was, now)| was != now)
}

/// Does the path of the running binary now lead to another file than the one this process runs?
/// Then `eye` in a run is newer than the daemon until the daemon restarts.
pub fn replaced() -> bool {
    let now = std::env::current_exe().ok();
    now.is_some_and(|p| replaced_since(RUNNING.get().copied().flatten(), &p))
}

fn link_to(dir: &Path, exe: &Path) -> Result<()> {
    if !exe.exists() {
        bail!(
            "{} is gone (replaced while eye ran?): restart the daemon so runs get this eye",
            exe.display()
        );
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    for name in [ALIAS_NAME, BIN_NAME] {
        let link = dir.join(name);
        if std::fs::read_link(&link).is_ok_and(|t| t == exe) {
            continue;
        }
        // a name of its own: two starters (the daemon and `run-now --wait`) can change the links
        // at the same moment
        let tmp = dir.join(format!(
            ".{name}.{}.{}.link",
            std::process::id(),
            next_temp()
        ));
        let linked =
            std::os::unix::fs::symlink(exe, &tmp).and_then(|()| std::fs::rename(&tmp, &link));
        if linked.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        linked.with_context(|| format!("linking {}", link.display()))?;
    }
    Ok(())
}

/// A number no other call of this process has had.
fn next_temp() -> u64 {
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

/// The `PATH` of a run: the folder of links, then the `PATH` eye has.
pub fn run_path(dir: &Path) -> OsString {
    path_before(dir, &crate::platform::search_path())
}

fn path_before(dir: &Path, rest: &OsStr) -> OsString {
    let mut path = dir.as_os_str().to_owned();
    path.push(":");
    path.push(rest);
    path
}

#[cfg(test)]
mod tests;
