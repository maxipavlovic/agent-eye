//! Small helpers for editing other programs' files safely: atomic writes with a mode, backups, a
//! line diff to show what changes, and 2-space JSON.

use anyhow::Result;
use serde_json::Value;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Pretty JSON (2-space) with a trailing newline, the form tools write their settings files in.
pub fn pretty_json(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s
}

/// Lines added/removed between two texts (LCS), as `+ n: text` / `- n: text`.
pub fn line_diff(old: &str, new: &str) -> Vec<String> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut t = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || t[i][j + 1] >= t[i + 1][j]) {
            out.push(format!("+ {:>4}: {}", j + 1, b[j]));
            j += 1;
        } else {
            out.push(format!("- {:>4}: {}", i + 1, a[i]));
            i += 1;
        }
    }
    out
}

/// Write a file only its owner can read (mode 0600), replacing any file there, also when it was
/// readable by others before.
pub fn write_private(p: &Path, text: &str) -> Result<()> {
    write_file(p, text.as_bytes(), 0o600)
}

/// Make an existing file readable by its owner only. Missing files and failures are ignored: this
/// tightens, it never blocks.
pub fn make_private(p: &Path) {
    if let Ok(md) = std::fs::metadata(p) {
        if md.permissions().mode() & 0o077 != 0 {
            let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Write `text` to `p` through a temp file and a rename, with the given mode. The temp file is
/// made new (a leftover of a crash, or a link put there, is removed, never written through), has
/// the mode from the start, and is on disk before the rename, so a power cut leaves the old file
/// or the new one, not an empty one.
pub fn write_file(p: &Path, text: &[u8], mode: u32) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("eye-tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)?;
    // the umask may have taken bits off the mode it was created with
    f.set_permissions(std::fs::Permissions::from_mode(mode))?;
    let done = f
        .write_all(text)
        .and_then(|()| f.sync_all())
        .and_then(|()| std::fs::rename(&tmp, p));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(done?)
}

/// `<name>.eye-backup-<timestamp>` next to `p`.
pub fn backup_name(p: &Path) -> PathBuf {
    let mut n = p.file_name().unwrap_or_default().to_os_string();
    n.push(format!(
        ".eye-backup-{}",
        jiff::ZonedDateTime::now().strftime("%Y%m%d-%H%M%S")
    ));
    p.with_file_name(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_shows_only_changes() {
        let d = line_diff("a\nb\nc\n", "a\nx\nb\nc\nd\n");
        assert_eq!(d, vec!["+    2: x".to_string(), "+    5: d".to_string()]);
    }

    #[test]
    fn private_files_lose_group_and_world_access() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("c.toml");
        std::fs::write(&p, "x").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        make_private(&p);
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_private(&p, "y").unwrap();
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "y");
        make_private(&t.path().join("missing"));
    }

    #[test]
    fn write_file_sets_the_mode_and_replaces_atomically() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a/b.txt");
        write_file(&p, b"one", 0o600).unwrap();
        write_file(&p, b"two", 0o600).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!p.with_extension("eye-tmp").exists());
    }

    #[test]
    fn write_file_never_writes_through_a_link_left_where_its_temp_file_goes() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("settings.json");
        let precious = t.path().join("precious");
        std::fs::write(&precious, "keep me").unwrap();
        std::os::unix::fs::symlink(&precious, p.with_extension("eye-tmp")).unwrap();
        write_file(&p, b"new", 0o644).unwrap();
        assert_eq!(std::fs::read_to_string(&precious).unwrap(), "keep me");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert!(!std::fs::symlink_metadata(&p).unwrap().is_symlink());
        // a temp file a crash left behind is replaced, whatever mode it had
        let tmp = p.with_extension("eye-tmp");
        std::fs::write(&tmp, "half").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_file(&p, b"newer", 0o600).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "newer");
        assert_eq!(
            std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!tmp.exists());
    }

    #[test]
    fn a_write_that_fails_leaves_the_old_file_and_no_temp_file() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a");
        write_file(&p, b"old", 0o600).unwrap();
        // a folder where the file would go: the rename fails
        let q = t.path().join("dir");
        std::fs::create_dir_all(q.join("inside")).unwrap();
        assert!(write_file(&q, b"x", 0o600).is_err());
        assert!(!q.with_extension("eye-tmp").exists());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "old");
    }
}
