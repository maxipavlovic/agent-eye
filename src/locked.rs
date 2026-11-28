//! Tests of what happens where eye cannot write: a folder with its files made read-only.

use std::fs::{set_permissions, Permissions};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Root writes anywhere, so a test that needs a folder nobody can write has nothing to check.
pub(crate) fn as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// A folder and the files directly in it made read-only, as a night run's sandbox leaves them; they
/// are given back when this is dropped, so the temp folder can be removed.
pub(crate) struct Locked(PathBuf);

impl Locked {
    pub(crate) fn new(dir: &Path) -> Locked {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_file() {
                set_permissions(e.path(), Permissions::from_mode(0o400)).unwrap();
            }
        }
        set_permissions(dir, Permissions::from_mode(0o500)).unwrap();
        Locked(dir.to_path_buf())
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        let _ = set_permissions(&self.0, Permissions::from_mode(0o700));
        for e in std::fs::read_dir(&self.0).into_iter().flatten().flatten() {
            if e.file_type().is_ok_and(|t| t.is_file()) {
                let _ = set_permissions(e.path(), Permissions::from_mode(0o600));
            }
        }
    }
}
