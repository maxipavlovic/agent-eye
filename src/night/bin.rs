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
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// The folder of links, inside eye's own.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.home.join("bin")
}

/// Make `dir` hold `eye` and `agent-eye`, both links to this binary, which is returned. A link
/// that already points there is left alone, so a run that starts while another reads it never
/// finds it missing.
pub fn refresh(dir: &Path) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("finding the running eye")?;
    link_to(dir, &exe)?;
    Ok(exe)
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
        let tmp = dir.join(format!(".{name}.link"));
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(exe, &tmp)
            .and_then(|()| std::fs::rename(&tmp, &link))
            .with_context(|| format!("linking {}", link.display()))?;
    }
    Ok(())
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
