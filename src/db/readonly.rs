//! Opening the database where it cannot be written: a night run's sandbox, a read-only folder.

use super::{Db, SCHEMA_VERSION};
use anyhow::{bail, Context, Result};
use rusqlite::{Connection, ErrorCode, OpenFlags};
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::Path;

/// What is said when something tries to write to a database that was opened read-only.
const READ_ONLY_HERE: &str =
    "eye.db is read-only here, so this command cannot change anything; run it where eye can write its folder";

/// Can this error be the answer to opening a file for writing where writing is not allowed?
fn is_unwritable(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem
    )
}

/// `path` with `suffix` after its name (`eye.db-wal`).
fn beside(path: &Path, suffix: &str) -> OsString {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s
}

/// A read-only connection. `immutable` tells SQLite that nothing changes the file, so it needs no
/// `-wal` or `-shm` file; that is only true when no log waits to be merged.
fn read_only_conn(path: &Path, immutable: bool) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI;
    let target = if immutable {
        let text = path.to_str().context("the path is not text")?;
        let escaped = text
            .replace('%', "%25")
            .replace('?', "%3f")
            .replace('#', "%23");
        format!("file:{escaped}?immutable=1")
    } else {
        path.to_string_lossy().into_owned()
    };
    let conn = Connection::open_with_flags(target, flags)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    // the first read is what opens the log, so a file that cannot be read fails here
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })?;
    Ok(conn)
}

impl Db {
    /// Open the database for a command that only reads. Where the file exists but this process
    /// cannot write it (a night run's sandbox, a read-only folder) it is opened read-only: no WAL
    /// pragma, no backup and no migration, so a database of another schema is refused, not
    /// upgraded, and a write fails plainly (see [`explain`]). Everywhere else this is [`Db::open`].
    ///
    /// Reading a WAL database read-only needs its `-wal` and `-shm` files, which another process
    /// that has it open (the daemon) keeps. With no `-wal` file, or an empty one, and no way to
    /// make them, the file alone is the whole database and is read as one that cannot change.
    /// With changes waiting in the `-wal` file and no way to read them it is refused, since the
    /// file alone would show old data without saying so.
    pub fn open_reading(path: &Path) -> Result<Db> {
        let unwritable = path.is_file()
            && std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .is_err_and(|e| is_unwritable(&e));
        if unwritable {
            Self::open_read_only(path)
        } else {
            Self::open(path)
        }
    }

    /// Whether this database was opened read-only: a command that would refresh it first skips that.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    fn open_read_only(path: &Path) -> Result<Db> {
        // SQLite says only "unable to open" for a file it cannot read: say what is wrong with it
        std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
        let conn = match read_only_conn(path, false) {
            Ok(c) => c,
            Err(_) if wal_is_empty(path) => read_only_conn(path, true)
                .with_context(|| format!("reading {}", path.display()))?,
            Err(e) if cannot_open_log(&e) => bail!(
                "{} has changes waiting in its -wal file, and reading them needs an -shm file beside it that cannot be made here: run eye once where it can write (SQLite said: {e})",
                path.display()
            ),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let db = Db {
            conn,
            read_only: true,
        };
        db.require_this_schema(path)?;
        Ok(db)
    }

    /// A read-only database cannot be upgraded, so it must already be this eye's.
    fn require_this_schema(&self, path: &Path) -> Result<()> {
        let version = match self.meta_get("schema_version") {
            Ok(v) => v.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
            Err(e) if format!("{e:#}").contains("no such table") => 0,
            Err(e) => return Err(e),
        };
        let shown = path.display();
        if version == SCHEMA_VERSION {
            Ok(())
        } else if version == 0 {
            bail!("{shown} is not set up yet, and it cannot be set up here: run eye once where it can write")
        } else if version < SCHEMA_VERSION {
            bail!("{shown} is from an older eye (schema {version}; this eye reads {SCHEMA_VERSION}) and cannot be upgraded here: run this eye once where it can write")
        } else {
            bail!("{shown} is from a newer eye (schema {version}; this eye reads {SCHEMA_VERSION}): use the newer eye here")
        }
    }
}

/// Is this what SQLite says when the log files it needs cannot be opened or made? Anything else
/// (a file that is not a database, one that cannot be read) is not about them.
fn cannot_open_log(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(f, _))
            if matches!(f.code, ErrorCode::CannotOpen | ErrorCode::ReadOnly)
    )
}

/// True when no change waits in the `-wal` file: it is not there, or it holds nothing.
fn wal_is_empty(path: &Path) -> bool {
    std::fs::metadata(beside(path, "-wal"))
        .ok()
        .is_none_or(|m| m.len() == 0)
}

/// An error as it is printed. A write that failed because the database was opened read-only says
/// so, in place of SQLite's "attempt to write a readonly database".
pub fn explain(e: &anyhow::Error) -> String {
    let read_only = e
        .chain()
        .filter_map(|c| c.downcast_ref::<rusqlite::Error>())
        .any(
            |r| matches!(r, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ReadOnly),
        );
    if read_only {
        READ_ONLY_HERE.to_string()
    } else {
        format!("{e:#}")
    }
}

#[cfg(test)]
mod tests;
