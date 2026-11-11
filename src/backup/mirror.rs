//! The optional mirror: each snapshot is copied into a folder the user names (an external disk, a
//! synced folder), with the same rotation there, and the transcript archive can follow.
//! The folder must exist already: when it does not (the disk is not plugged in) nothing is made,
//! because creating `/Volumes/Disk/...` while the disk is away would fill the boot disk.

use crate::config::BackupCfg;
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

const STATE_FILE: &str = "mirror.state";

/// How the last copy to the mirror ended, as `eye doctor` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Copied at this Unix ms.
    Ok(i64),
    /// Did not work, at this Unix ms, for this reason.
    Failed(i64, String),
}

fn state_path(paths: &Paths) -> PathBuf {
    super::dir(paths).join(STATE_FILE)
}

/// What the last copy did; None when none was made (or the mirror is new).
pub fn state(paths: &Paths) -> Option<State> {
    let text = std::fs::read_to_string(state_path(paths)).ok()?;
    let mut parts = text.trim().splitn(3, ' ');
    let kind = parts.next()?;
    let at: i64 = parts.next()?.parse().ok()?;
    match kind {
        "ok" => Some(State::Ok(at)),
        "failed" => Some(State::Failed(at, parts.next().unwrap_or_default().into())),
        _ => None,
    }
}

fn record(paths: &Paths, state: &State) {
    let line = match state {
        State::Ok(at) => format!("ok {at}"),
        State::Failed(at, why) => format!("failed {at} {}", why.replace('\n', " ")),
    };
    let _ = crate::files::write_private(&state_path(paths), &line);
}

/// Copy `newest` (a snapshot just made) to the mirror, rotate there, and copy the archive when
/// asked to. Without a mirror it does nothing; the outcome is kept for `eye doctor`.
pub fn sync(paths: &Paths, cfg: &BackupCfg, newest: &Path) {
    let mirror = match cfg.mirror_dir() {
        Ok(Some(m)) => m,
        Ok(None) => return,
        Err(why) => {
            record(paths, &State::Failed(crate::util::now_ms(), why));
            return;
        }
    };
    let result = copy_snapshot(&mirror, newest, cfg).and_then(|()| {
        if cfg.mirror_archive {
            copy_archive(&paths.home.join("archive"), &mirror.join("archive"))?;
        }
        Ok(())
    });
    let now = crate::util::now_ms();
    record(
        paths,
        &match result {
            Ok(()) => State::Ok(now),
            Err(e) => State::Failed(now, format!("{e:#}")),
        },
    );
}

fn copy_snapshot(mirror: &Path, newest: &Path, cfg: &BackupCfg) -> Result<()> {
    if !mirror.is_dir() {
        bail!(
            "the folder {} does not exist (a disk not plugged in?)",
            mirror.display()
        );
    }
    let name = newest.file_name().context("a snapshot without a name")?;
    let dest = mirror.join(name);
    let tmp = mirror.join(format!(".eye-{}.copy.tmp", std::process::id()));
    let copied = std::fs::copy(newest, &tmp)
        .with_context(|| format!("copying to {}", mirror.display()))
        .and_then(|_| {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            std::fs::rename(&tmp, &dest).with_context(|| format!("writing {}", dest.display()))
        });
    if copied.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    copied?;
    super::rotate(mirror, cfg);
    Ok(())
}

/// Copy the files of `from` into `to` (folders as they are in `from`) when they are not there yet
/// or their size or modified time differ. Nothing in `to` is ever deleted.
pub fn copy_archive(from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)?.flatten() {
        let (src, dest) = (entry.path(), to.join(entry.file_name()));
        let md = entry.metadata()?;
        if md.is_dir() {
            copy_archive(&src, &dest)?;
        } else if md.is_file() && changed(&md, &dest) {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(to)
                .with_context(|| format!("creating {}", to.display()))?;
            copy_file(&src, &dest, &md)?;
        }
    }
    Ok(())
}

fn changed(src: &std::fs::Metadata, dest: &Path) -> bool {
    match std::fs::metadata(dest) {
        Ok(d) => d.len() != src.len() || d.modified().ok() != src.modified().ok(),
        Err(_) => true,
    }
}

/// Copy through a temp file so a half copy is never taken for a finished one, and give the copy
/// the original's modified time, which is how a later run knows it is unchanged.
fn copy_file(src: &Path, dest: &Path, md: &std::fs::Metadata) -> Result<()> {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".eye-tmp");
    let tmp = dest.with_file_name(name);
    let done = std::fs::copy(src, &tmp)
        .and_then(|_| {
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            std::fs::File::options()
                .write(true)
                .open(&tmp)?
                .set_modified(md.modified()?)?;
            std::fs::rename(&tmp, dest)
        })
        .with_context(|| format!("copying {}", src.display()));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}
