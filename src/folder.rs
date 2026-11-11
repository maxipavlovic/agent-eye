//! Where an idea starts: does its folder exist, and may eye create it.
//!
//! Eye only ever creates a folder inside the user's home folder, and only a plain one.

use anyhow::{Context, Result};
use std::ffi::CString;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// What a typed folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderState {
    /// A folder is there.
    Exists,
    /// Nothing is there; `creatable` says whether eye may make it.
    Missing { creatable: bool },
    /// A file is in the way: at the path or at one of its parents.
    NotAFolder,
    /// Not a usable folder name or not readable; the reason is for people.
    Unusable(String),
}

impl FolderState {
    /// The word the API uses: `ok`, `missing`, `not_a_folder` or `unusable`.
    pub fn code(&self) -> &'static str {
        match self {
            FolderState::Exists => "ok",
            FolderState::Missing { .. } => "missing",
            FolderState::NotAFolder => "not_a_folder",
            FolderState::Unusable(_) => "unusable",
        }
    }

    /// One plain line for a form or a terminal; none when the folder is fine.
    pub fn note(&self) -> Option<String> {
        match self {
            FolderState::Exists => None,
            FolderState::Missing { creatable: true } => {
                Some("Folder does not exist yet; Start will create it".into())
            }
            FolderState::Missing { creatable: false } => Some(
                "Folder does not exist, and eye only creates folders inside your home folder"
                    .into(),
            ),
            FolderState::NotAFolder => Some("Not a folder".into()),
            FolderState::Unusable(why) => Some(why.clone()),
        }
    }
}

/// A start that needs its folder made first, and was not allowed to.
#[derive(Debug)]
pub struct FolderMissing(pub String);

impl std::fmt::Display for FolderMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "the folder {} does not exist yet", self.0)
    }
}

impl std::error::Error for FolderMissing {}

fn gone(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

fn unreadable(path: &Path, e: &std::io::Error) -> FolderState {
    FolderState::Unusable(format!("Cannot look at {}: {e}", path.display()))
}

fn writable(dir: &Path) -> bool {
    CString::new(dir.as_os_str().as_bytes())
        // SAFETY: `c` is a valid NUL-terminated path that outlives the call.
        .is_ok_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) } == 0)
}

/// Classify a folder as typed (a leading `~` is the home folder). `root` is the folder eye may
/// create inside, normally the home folder.
pub fn classify(root: Option<&Path>, raw: &str) -> FolderState {
    let typed = raw.trim();
    if typed.is_empty() {
        return FolderState::Unusable("No folder given".into());
    }
    if typed.chars().any(char::is_control) {
        return FolderState::Unusable("A folder name cannot contain control characters".into());
    }
    let path = crate::config::expand_tilde(typed);
    if !path.is_absolute() {
        return FolderState::Unusable("Give the full path, or start with ~".into());
    }
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => return FolderState::Exists,
        Ok(_) => return FolderState::NotAFolder,
        Err(e) if gone(&e) => {}
        Err(e) => return unreadable(&path, &e),
    }
    for parent in path.ancestors().skip(1) {
        match std::fs::metadata(parent) {
            Ok(m) if m.is_dir() => {
                let plain = !path.components().any(|c| c == Component::ParentDir);
                let creatable = plain && inside(root, parent) && writable(parent);
                return FolderState::Missing { creatable };
            }
            Ok(_) => return FolderState::NotAFolder,
            Err(e) if gone(&e) => {}
            Err(e) => return unreadable(parent, &e),
        }
    }
    FolderState::Missing { creatable: false }
}

/// Is the existing folder `dir` inside `root` once links are followed?
fn inside(root: Option<&Path>, dir: &Path) -> bool {
    match (root.and_then(|r| r.canonicalize().ok()), dir.canonicalize()) {
        (Some(r), Ok(d)) => d.starts_with(r),
        _ => false,
    }
}

/// Make the folder (and the missing folders above it) when `classify` says eye may.
pub fn create(root: Option<&Path>, raw: &str) -> Result<PathBuf> {
    let state = classify(root, raw);
    anyhow::ensure!(
        state == FolderState::Missing { creatable: true },
        "{}",
        state
            .note()
            .unwrap_or_else(|| "the folder is already there".into())
    );
    let path = crate::config::expand_tilde(raw.trim());
    std::fs::create_dir_all(&path).with_context(|| format!("creating {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(root: &Path, p: &Path) -> FolderState {
        classify(Some(root), &p.to_string_lossy())
    }

    #[test]
    fn a_folder_that_is_there_exists() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(s(tmp.path(), tmp.path()), FolderState::Exists);
        assert_eq!(FolderState::Exists.note(), None);
        assert_eq!(FolderState::Exists.code(), "ok");
    }

    #[test]
    fn a_missing_folder_inside_home_can_be_made() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a/b/c");
        assert_eq!(s(tmp.path(), &p), FolderState::Missing { creatable: true });
        let made = create(Some(tmp.path()), &p.to_string_lossy()).unwrap();
        assert!(made.is_dir());
        assert_eq!(s(tmp.path(), &p), FolderState::Exists);
    }

    #[test]
    fn a_missing_folder_outside_home_cannot_be_made() {
        let home = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let p = other.path().join("new");
        assert_eq!(
            s(home.path(), &p),
            FolderState::Missing { creatable: false }
        );
        assert!(create(Some(home.path()), &p.to_string_lossy()).is_err());
        assert!(!p.exists());
        assert_eq!(
            classify(None, &p.to_string_lossy()),
            FolderState::Missing { creatable: false }
        );
    }

    #[test]
    fn dot_dot_never_counts_as_inside_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let p = home.join("../escaped");
        assert_eq!(s(&home, &p), FolderState::Missing { creatable: false });
        assert!(create(Some(&home), &p.to_string_lossy()).is_err());
        assert!(!tmp.path().join("escaped").exists());
    }

    #[test]
    fn a_link_out_of_home_does_not_let_a_folder_be_made() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, out) = (tmp.path().join("home"), tmp.path().join("out"));
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&out).unwrap();
        std::os::unix::fs::symlink(&out, home.join("link")).unwrap();
        let p = home.join("link/new");
        assert_eq!(s(&home, &p), FolderState::Missing { creatable: false });
    }

    #[test]
    fn a_file_in_the_way_is_not_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("file");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(s(tmp.path(), &f), FolderState::NotAFolder);
        assert_eq!(
            s(tmp.path(), &f.join("sub/deeper")),
            FolderState::NotAFolder
        );
        assert_eq!(
            FolderState::NotAFolder.note().as_deref(),
            Some("Not a folder")
        );
        assert!(create(Some(tmp.path()), &f.join("sub").to_string_lossy()).is_err());
    }

    #[test]
    fn a_read_only_parent_cannot_hold_a_new_folder() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let ro = tmp.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o500)).unwrap();
        let state = s(tmp.path(), &ro.join("new"));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o700)).unwrap();
        // the owner of a test run may be root, who can write anywhere
        let root_user = crate::platform::uid() == 0;
        assert_eq!(
            state,
            FolderState::Missing {
                creatable: root_user
            }
        );
    }

    #[test]
    fn typed_nonsense_is_unusable() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["", "   ", "relative/dir", "/tmp/a\nb"] {
            let st = classify(Some(tmp.path()), bad);
            assert!(matches!(st, FolderState::Unusable(_)), "{bad:?}: {st:?}");
            assert_eq!(st.code(), "unusable");
            assert!(st.note().is_some());
        }
    }

    #[test]
    fn a_tilde_is_the_home_folder() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(classify(Some(Path::new(&home)), "~"), FolderState::Exists);
        let st = classify(Some(Path::new(&home)), "~/definitely-not-here-0f3a9/x");
        assert!(matches!(st, FolderState::Missing { .. }), "{st:?}");
    }

    #[test]
    fn the_notes_read_as_plain_lines() {
        let up = FolderState::Missing { creatable: true };
        assert_eq!(up.code(), "missing");
        assert!(up.note().unwrap().contains("Start will create it"));
        let no = FolderState::Missing { creatable: false };
        assert!(no.note().unwrap().contains("inside your home folder"));
    }
}
