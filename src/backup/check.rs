//! The backup lines of `eye doctor`.

use super::mirror::{self, State};
use super::restore::quick_check;
use crate::agents::Check;
use crate::config::Config;
use crate::paths::Paths;
use crate::util::ago;
use rusqlite::{Connection, OpenFlags};

/// Automatic snapshots that are older than this are worth a warning.
const STALE_MS: i64 = 3 * 24 * 3600 * 1000;

/// Snapshots (count, size, newest), the mirror, and the integrity of the live database. Changes
/// nothing.
pub fn checks(paths: &Paths, cfg: &Config, now_ms: i64) -> Vec<Check> {
    let mut out = vec![];
    let all = super::list(&super::dir(paths));
    let total: u64 = all.iter().map(|e| e.bytes).sum();
    let on = cfg.backup.enabled;
    out.push(match all.first() {
        Some(newest) => {
            let age = (now_ms - newest.stamp_ms(), ago(now_ms, newest.stamp_ms()));
            let line = format!(
                "backups: {} snapshots, {:.1} MB, newest {} ({}){}",
                all.len(),
                total as f64 / 1e6,
                age.1,
                newest.reason.label(),
                if on {
                    ""
                } else {
                    "; automatic snapshots are off"
                }
            );
            let stale = on && age.0 > STALE_MS;
            if stale {
                Check::warn(format!(
                    "{line}; automatic ones should be daily (is the daemon running?)"
                ))
            } else {
                Check::ok(line)
            }
        }
        None if !on => {
            Check::ok("backups: automatic snapshots are off ([backup] enabled = false); none yet")
        }
        None if paths.db_path().exists() => {
            Check::warn("backups: none yet (the daemon makes one daily, or run `eye backup`)")
        }
        None => Check::ok("backups: none yet"),
    });
    out.push(mirror_check(paths, cfg, now_ms));
    out.extend(live_check(paths));
    out
}

fn mirror_check(paths: &Paths, cfg: &Config, now_ms: i64) -> Check {
    let dir = match cfg.backup.mirror_dir() {
        Ok(Some(dir)) => dir,
        Ok(None) => return Check::ok("backup mirror: off"),
        Err(why) => return Check::warn(format!("backup mirror: {why}; no copies are made")),
    };
    let shown = dir.display();
    let last = mirror::state(paths);
    let last_ok = match &last {
        Some(State::Ok(t)) => format!("last copy {}", ago(now_ms, *t)),
        _ => "no copy made yet".to_string(),
    };
    if !dir.is_dir() {
        return Check::warn(format!(
            "backup mirror: folder {shown} is missing ({last_ok})"
        ));
    }
    match last {
        Some(State::Failed(t, why)) => Check::warn(format!(
            "backup mirror: the last copy to {shown} failed {}: {why}",
            ago(now_ms, t)
        )),
        _ => Check::ok(format!("backup mirror: {shown} ({last_ok})")),
    }
}

/// `PRAGMA quick_check` of the live database, opened read-only.
fn live_check(paths: &Paths) -> Option<Check> {
    let db = paths.db_path();
    if !db.exists() {
        return None;
    }
    let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    Some(match quick_check(&conn) {
        None => Check::ok("database check: ok"),
        Some(found) => Check::warn(format!(
            "database check failed ({found}); `eye backup restore latest` puts the newest snapshot back"
        )),
    })
}
