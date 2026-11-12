//! The daemon's own binary on disk. `brew upgrade`, `cargo install` or a new copy replaces it while
//! the daemon still runs the old code; the daemon notices and starts the new binary in its place.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A binary as it is on disk: the file every link ends at, and that file's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    file: PathBuf,
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

fn seen(path: &Path) -> Option<Seen> {
    let file = std::fs::canonicalize(path).ok()?;
    let m = std::fs::metadata(&file).ok()?;
    Some(Seen {
        file,
        dev: m.dev(),
        ino: m.ino(),
        len: m.len(),
        mtime: m.mtime(),
        mtime_nsec: m.mtime_nsec(),
    })
}

/// Watches the path a binary was started from.
pub struct Watch {
    path: PathBuf,
    start: Seen,
    pending: Option<Seen>,
}

impl Watch {
    /// The binary this process was started from: the path the service manager ran (the first
    /// argument), or the one `PATH` finds for a bare name. None when it cannot be found.
    pub fn of_this_process() -> Option<Watch> {
        let arg0 = PathBuf::from(std::env::args_os().next()?);
        let path = if arg0.components().count() > 1 {
            std::path::absolute(arg0).ok()?
        } else {
            crate::util::find_in_path(arg0.to_str()?)?
        };
        Watch::new(path)
    }

    pub fn new(path: PathBuf) -> Option<Watch> {
        let start = seen(&path)?;
        Some(Watch {
            path,
            start,
            pending: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Has the binary been replaced, the same way at two looks in a row? A file still being
    /// written, or a link in the middle of a swap, waits for the next look; a missing one too.
    pub fn upgraded(&mut self) -> bool {
        match seen(&self.path) {
            Some(now) if now != self.start => {
                let settled = self.pending.as_ref() == Some(&now);
                self.pending = Some(now);
                settled
            }
            _ => {
                self.pending = None;
                false
            }
        }
    }
}

/// Start the binary at `path` in this process's place, with the same arguments, environment and
/// pid. Returns only when that fails; the daemon then exits with the error, and the service
/// manager starts it again.
pub fn restart(path: &Path) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(path)
        .args(std::env::args_os().skip(1))
        .exec();
    anyhow::Error::new(err).context(format!("starting the upgraded {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Write `text` to `path` the way installers do: a new file renamed over the old one.
    fn replace(path: &Path, text: &str) {
        let tmp = path.with_extension("new");
        std::fs::write(&tmp, text).unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    #[test]
    fn an_untouched_binary_is_never_an_upgrade() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("agent-eye");
        std::fs::write(&bin, "v1").unwrap();
        let mut w = Watch::new(bin).unwrap();
        for _ in 0..3 {
            assert!(!w.upgraded());
        }
    }

    #[test]
    fn a_replaced_binary_counts_at_the_second_look() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("agent-eye");
        std::fs::write(&bin, "v1").unwrap();
        let mut w = Watch::new(bin.clone()).unwrap();
        replace(&bin, "v2");
        assert!(!w.upgraded(), "the first look only notes it");
        assert!(w.upgraded());
    }

    #[test]
    fn a_link_moved_to_a_new_version_is_an_upgrade() {
        // Homebrew: opt/agent-eye points at the newest keg after `brew upgrade`
        let t = tempfile::tempdir().unwrap();
        for v in ["1", "2"] {
            std::fs::create_dir_all(t.path().join(v)).unwrap();
            std::fs::write(t.path().join(v).join("agent-eye"), "same size").unwrap();
        }
        let opt = t.path().join("opt");
        symlink(t.path().join("1"), &opt).unwrap();
        let mut w = Watch::new(opt.join("agent-eye")).unwrap();
        std::fs::remove_file(&opt).unwrap();
        symlink(t.path().join("2"), &opt).unwrap();
        assert!(!w.upgraded());
        assert!(w.upgraded());
    }

    #[test]
    fn a_binary_that_is_gone_or_still_changing_waits() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("agent-eye");
        std::fs::write(&bin, "v1").unwrap();
        let mut w = Watch::new(bin.clone()).unwrap();
        std::fs::remove_file(&bin).unwrap();
        assert!(!w.upgraded());
        assert!(!w.upgraded(), "nothing to start");
        replace(&bin, "v2, half");
        assert!(!w.upgraded());
        replace(&bin, "v2, whole");
        assert!(!w.upgraded(), "it changed again since the last look");
        assert!(w.upgraded());
    }
}
