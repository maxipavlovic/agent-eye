//! Small helpers for editing other programs' files safely: atomic writes with a mode, backups, a
//! line diff to show what changes, and 2-space JSON.

use anyhow::Result;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
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

/// Write `text` to `p` through a temp file and a rename, with the given mode.
pub fn write_file(p: &Path, text: &[u8], mode: u32) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("eye-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// `<name>.eye-backup-<timestamp>` next to `p`.
pub fn backup_name(p: &Path) -> PathBuf {
    let mut n = p.file_name().unwrap_or_default().to_os_string();
    n.push(format!(
        ".eye-backup-{}",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
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
}
