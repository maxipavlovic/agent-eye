//! Putting a snapshot back as the database. The snapshot is decompressed and checked first; the
//! current database is snapshotted as `pre-restore`; the daemon is stopped and holds nothing open
//! while the file is replaced, together with the removal of its old `-wal` and `-shm` files (a
//! stale WAL applied to the restored file would corrupt it).

use super::{Entry, Reason};
use crate::archive::decompress_file;
use crate::config::Config;
use crate::daemon::Lock;
use crate::db::SCHEMA_VERSION;
use crate::install::{pause_daemon, resume_daemon, Opts};
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the daemon gets to let go of the lock after it was told to stop.
const LOCK_WAIT: Duration = Duration::from_secs(10);

/// A snapshot decompressed to a temp file and found sound. The temp files go with it.
#[derive(Debug)]
pub struct Prepared {
    pub entry: Entry,
    /// The schema version inside the snapshot.
    pub schema: i64,
    file: PathBuf,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let mut name = self.file.as_os_str().to_os_string();
            name.push(suffix);
            let _ = std::fs::remove_file(name);
        }
    }
}

/// What a restore did.
#[derive(Debug)]
pub struct Applied {
    /// The copy of the database as it was; None when it was missing or unreadable.
    pub pre_restore: Option<PathBuf>,
    /// The daemon was stopped for it and started again.
    pub daemon_restarted: bool,
    /// What starting the daemon again said, when it did not work.
    pub restart_failed: Option<String>,
}

/// The snapshot called `name` (with or without `.db.zst`), or the newest one for `latest`.
pub fn find(dir: &Path, name: &str) -> Result<Entry> {
    let all = super::list(dir);
    let found = if name == "latest" {
        all.into_iter().next()
    } else {
        let name = name.strip_suffix(".db.zst").unwrap_or(name);
        all.into_iter()
            .find(|e| e.name.strip_suffix(".db.zst") == Some(name))
    };
    found.with_context(|| format!("no snapshot called {name} (see `eye backup ls`)"))
}

/// The schema version a database file says it has; an error when it is not an eye database.
pub fn schema_of(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse().ok())
    .context("it is not an eye database (no schema version)")
}

/// `PRAGMA quick_check` of the database behind `conn`: None when it is fine, else what it found.
pub fn quick_check(conn: &Connection) -> Option<String> {
    match conn.query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0)) {
        Ok(s) if s == "ok" => None,
        Ok(s) => Some(s),
        Err(e) => Some(e.to_string()),
    }
}

/// Decompress `entry` to a temp file next to the snapshots, unchecked.
fn unpack(paths: &Paths, entry: Entry) -> Result<Prepared> {
    let file = super::dir(paths).join(format!(".eye-restore-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let prepared = Prepared {
        entry,
        schema: 0,
        file,
    };
    decompress_file(&prepared.entry.path, &prepared.file)
        .with_context(|| format!("reading {}", prepared.entry.name))?;
    Ok(prepared)
}

fn open_unpacked(p: &Prepared) -> Result<Connection> {
    Connection::open_with_flags(&p.file, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("opening the snapshot")
}

/// Decompress `entry` and check it: `quick_check` must say ok, and its schema must not be newer
/// than this build knows.
pub fn prepare(paths: &Paths, entry: Entry) -> Result<Prepared> {
    let mut prepared = unpack(paths, entry)?;
    let conn = open_unpacked(&prepared)?;
    if let Some(found) = quick_check(&conn) {
        bail!("{} is damaged ({found})", prepared.entry.name);
    }
    prepared.schema = schema_of(&conn).with_context(|| prepared.entry.name.clone())?;
    if prepared.schema > SCHEMA_VERSION {
        bail!(
            "{} was made by a newer eye (schema {}, this one knows {SCHEMA_VERSION}); update eye first",
            prepared.entry.name,
            prepared.schema
        );
    }
    Ok(prepared)
}

/// The schema version inside a snapshot, for the listing; None when it cannot be read.
pub fn schema_version(paths: &Paths, entry: &Entry) -> Option<i64> {
    let p = unpack(paths, entry.clone()).ok()?;
    schema_of(&open_unpacked(&p).ok()?).ok()
}

/// Take the daemon's lock, waiting for a daemon that is stopping to let go of it.
fn lock(paths: &Paths) -> Result<Lock> {
    let until = Instant::now() + LOCK_WAIT;
    loop {
        match Lock::acquire(paths) {
            Ok(l) => return Ok(l),
            Err(e) if Instant::now() >= until => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Replace the database with the prepared snapshot. Order: copy of the current database, stop the
/// daemon (when it runs: through the service, and a daemon started by hand stops the restore),
/// replace the files while holding the daemon's lock, start the daemon again.
pub fn apply(o: &Opts, cfg: &Config, prepared: &Prepared) -> Result<Applied> {
    let paths = Paths {
        home: o.eye_home.clone(),
    };
    let db = paths.db_path();
    let pre_restore = pre_restore_copy(&paths, cfg, &db)?;
    let stopped = pause_daemon(o).map_err(anyhow::Error::msg)?;
    let swapped = lock(&paths).and_then(|_lock| swap(&db, &prepared.file));
    let (daemon_restarted, restart_failed) = if stopped {
        let said = resume_daemon(o);
        (said.ok, (!said.ok).then_some(said.text))
    } else {
        (false, None)
    };
    swapped?;
    Ok(Applied {
        pre_restore,
        daemon_restarted,
        restart_failed,
    })
}

/// Snapshot the database as it is now, as `pre-restore`. A database that is missing or fails its
/// check is not worth a copy (that is often why a person restores); one that is sound but cannot
/// be copied stops the restore, since nothing would be left to go back to.
fn pre_restore_copy(paths: &Paths, cfg: &Config, db: &Path) -> Result<Option<PathBuf>> {
    if !db.exists() {
        return Ok(None);
    }
    let Ok(conn) = Connection::open(db) else {
        return Ok(None);
    };
    let _ = conn.busy_timeout(Duration::from_secs(5));
    if quick_check(&conn).is_some() {
        return Ok(None);
    }
    let snap = super::take(&conn, paths, &cfg.backup, &Reason::PreRestore)
        .context("could not keep a copy of the current database, so nothing was changed")?;
    Ok(Some(snap.path))
}

/// Remove the old `-wal` and `-shm` files, then move the checked copy over the database.
fn swap(db: &Path, new: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        match std::fs::remove_file(&name) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing {}", Path::new(&name).display()))
            }
            _ => {}
        }
    }
    std::fs::rename(new, db).with_context(|| format!("replacing {}", db.display()))
}
