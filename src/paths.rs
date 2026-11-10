use anyhow::{Context, Result};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Where eye keeps its own data (`~/.agent-eye`). Where an agent keeps its files is the agent's
/// business (see `[agents.<id>] dir`).
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
}

/// Before the rename the data lived in `~/.claude-eye`. When `~/.agent-eye` does not exist yet,
/// move the old directory there and leave a symlink behind, so the old binary keeps working.
/// Returns true when it moved. Never fails the caller: a failed move just leaves the old dir.
pub fn migrate_old_home(user_home: &Path) -> bool {
    let new = user_home.join(".agent-eye");
    let old = user_home.join(".claude-eye");
    let old_is_dir = std::fs::symlink_metadata(&old).is_ok_and(|m| m.is_dir());
    if new.symlink_metadata().is_ok() || !old_is_dir {
        return false;
    }
    if std::fs::rename(&old, &new).is_err() {
        return false;
    }
    let _ = std::os::unix::fs::symlink(&new, &old);
    true
}

impl Paths {
    /// An explicit folder wins; otherwise `EYE_HOME`, then `~/.agent-eye`.
    pub fn resolve(home: Option<PathBuf>) -> Result<Paths> {
        let home = match home.or_else(|| env_path("EYE_HOME")) {
            Some(p) => p,
            None => {
                let h = std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .context("HOME is not set")?;
                migrate_old_home(&h);
                h.join(".agent-eye")
            }
        };
        Ok(Paths { home })
    }

    /// Create the eye home, readable by its owner only (0700). A home made earlier with wider
    /// rights is tightened.
    pub fn ensure_home(&self) -> Result<()> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.home)
            .with_context(|| format!("creating {}", self.home.display()))?;
        let md = std::fs::metadata(&self.home)
            .with_context(|| format!("reading {}", self.home.display()))?;
        if md.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(&self.home, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("securing {}", self.home.display()))?;
        }
        Ok(())
    }

    /// The database file.
    pub fn db_path(&self) -> PathBuf {
        self.home.join("eye.db")
    }
    /// The config file.
    pub fn config_path(&self) -> PathBuf {
        self.home.join("config.toml")
    }
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_home_moves_once_and_leaves_a_symlink() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join(".claude-eye");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("eye.db"), "x").unwrap();
        assert!(migrate_old_home(t.path()));
        let new = t.path().join(".agent-eye");
        assert_eq!(std::fs::read_to_string(new.join("eye.db")).unwrap(), "x");
        assert_eq!(std::fs::read_link(&old).unwrap(), new);
        // through the symlink the old path still reads the data
        assert_eq!(std::fs::read_to_string(old.join("eye.db")).unwrap(), "x");
        // second run: nothing to do
        assert!(!migrate_old_home(t.path()));
    }

    #[test]
    fn the_home_is_private_even_when_it_existed_with_wider_rights() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("eye");
        let paths = Paths { home: home.clone() };
        paths.ensure_home().unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&home), 0o700);
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
        paths.ensure_home().unwrap();
        assert_eq!(mode(&home), 0o700);
    }

    #[test]
    fn nothing_moves_when_new_exists_or_old_missing() {
        let t = tempfile::tempdir().unwrap();
        assert!(!migrate_old_home(t.path()));
        std::fs::create_dir_all(t.path().join(".claude-eye")).unwrap();
        std::fs::create_dir_all(t.path().join(".agent-eye")).unwrap();
        assert!(!migrate_old_home(t.path()));
        assert!(t.path().join(".claude-eye").is_dir());
    }
}
