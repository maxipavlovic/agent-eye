//! Snapshots of the database: a compressed, consistent copy made through SQLite (never a plain
//! file copy, which can miss what is still in the `-wal` file), kept in `backups/` under the eye
//! home. What a snapshot is and why it was made is in its file name,
//! `eye-<YYYYMMDD-HHMMSS>-<reason>.db.zst`; nothing about backups is stored in the database.

mod check;
mod mirror;
mod names;
mod restore;

pub use check::checks;
pub use names::{Entry, Reason};
pub use restore::{apply, find, is_snapshot, prepare, schema_version, Applied, Prepared};

use crate::archive::compress_file;
use crate::config::{BackupCfg, Config};
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

/// A daily snapshot is made when the newest one is older than this.
const DAILY_EVERY_MS: i64 = 24 * 3600 * 1000;

/// A snapshot that was made.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    /// Bytes on disk, compressed.
    pub bytes: u64,
}

/// Where the snapshots are kept.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.home.join("backups")
}

/// The snapshots in `dir`, newest first. Files that do not follow eye's naming are not listed.
pub fn list(dir: &Path) -> Vec<Entry> {
    let mut all: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Entry::from_path(&e.path()))
        .collect();
    all.sort_by(|a, b| b.stamp.cmp(&a.stamp).then_with(|| b.name.cmp(&a.name)));
    all
}

/// Create a folder only its owner can use; one made earlier with wider rights is tightened.
fn private_dir(dir: &Path) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
        .with_context(|| format!("securing {}", dir.display()))
}

/// Removes the files it was given when dropped, so a failed snapshot leaves no temp file.
struct Temps(Vec<PathBuf>);

impl Drop for Temps {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Snapshot the database behind `conn` into `dir` for `reason`. The copy is built in temp files
/// (named by process, so two processes never share one), compressed, and renamed into place; two
/// processes that make the same name at the same second replace each other's equal copy.
pub fn snapshot(conn: &Connection, dir: &Path, reason: &Reason) -> Result<Snapshot> {
    private_dir(dir)?;
    let tag = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let raw = dir.join(format!(".eye-{tag}.db.tmp"));
    let packed = dir.join(format!(".eye-{tag}.zst.tmp"));
    let _temps = Temps(vec![raw.clone(), packed.clone()]);
    conn.execute("VACUUM INTO ?1", [raw.to_string_lossy()])
        .context("copying the database")?;
    compress_file(&raw, &packed).context("compressing the snapshot")?;
    let name = names::file_name(chrono::Local::now().naive_local(), reason);
    let path = dir.join(name);
    std::fs::rename(&packed, &path).with_context(|| format!("writing {}", path.display()))?;
    let bytes = std::fs::metadata(&path)?.len();
    Ok(Snapshot { path, bytes })
}

/// Snapshot, then rotate, then copy to the mirror when there is one. The snapshot is the part
/// that must work; a mirror that fails is noted for `eye doctor` and does not fail the snapshot.
pub fn take(
    conn: &Connection,
    paths: &Paths,
    cfg: &BackupCfg,
    reason: &Reason,
) -> Result<Snapshot> {
    let dir = dir(paths);
    let snap = snapshot(conn, &dir, reason)?;
    rotate(&dir, cfg);
    mirror::sync(paths, cfg, &snap.path);
    Ok(snap)
}

/// The daemon's daily snapshot: made when automatic snapshots are on and no `daily` one is less
/// than a day old. Cheap when not due (one folder listing).
pub fn daily_if_due(
    conn: &Connection,
    paths: &Paths,
    cfg: &Config,
    now_ms: i64,
) -> Result<Option<Snapshot>> {
    if !cfg.backup.enabled {
        return Ok(None);
    }
    let newest = list(&dir(paths))
        .into_iter()
        .find(|e| e.reason == Reason::Daily)
        .and_then(|e| e.stamp_ms());
    if newest.is_some_and(|t| now_ms - t < DAILY_EVERY_MS) {
        return Ok(None);
    }
    take(conn, paths, &cfg.backup, &Reason::Daily).map(Some)
}

/// Before `migrate` changes an older database: snapshot it as `pre-v<target>`. Never blocks the
/// tool: when the snapshot cannot be made it says why on standard error and the database opens.
/// A new or empty database, one already at `target`, and `enabled = false` make none.
pub fn before_upgrade(conn: &Connection, db_path: &Path, target: i64) {
    let Some(home) = db_path.parent() else { return };
    let paths = Paths {
        home: home.to_path_buf(),
    };
    let cfg = crate::install::read_config(&paths);
    if !cfg.backup.enabled || !needs_upgrade(conn, target) {
        return;
    }
    if let Err(e) = take(conn, &paths, &cfg.backup, &Reason::PreUpgrade(target)) {
        eprintln!("eye: no backup before the upgrade to schema {target}: {e:#}");
    }
}

/// Does the database hold data of a schema older than `target`? An empty one is not worth a copy.
fn needs_upgrade(conn: &Connection, target: i64) -> bool {
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let version = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|v| v.parse::<i64>().ok());
    match version {
        Some(v) => v < target,
        // From before the version was kept: old when it has sessions.
        None => conn
            .query_row("SELECT 1 FROM session LIMIT 1", [], |_| Ok(()))
            .is_ok(),
    }
}

/// Delete the snapshots in `dir` that no rule keeps, and temp files a crash left behind. Only
/// files that follow eye's own naming are touched; a failure to delete one is not an error.
pub fn rotate(dir: &Path, cfg: &BackupCfg) {
    let keep = names::keep(&list(dir), cfg);
    for e in list(dir) {
        if !keep.contains(&e.name) {
            let _ = std::fs::remove_file(&e.path);
        }
    }
    names::remove_stale_temps(dir);
}

#[cfg(test)]
mod tests;
