use super::*;
use crate::locked::{as_root, Locked};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A folder with a database in it.
struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Home {
        Home(tempfile::tempdir().unwrap())
    }

    fn db(&self) -> PathBuf {
        self.0.path().join("eye.db")
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.0.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

/// A database with two sessions. It is closed when the result is dropped, which leaves no `-wal`
/// or `-shm` file behind.
fn filled(path: &Path) -> Db {
    let db = Db::open(path).unwrap();
    db.conn
        .execute(
            "INSERT INTO session(id, title) VALUES('s1', 'first'), ('s2', 'second')",
            [],
        )
        .unwrap();
    db
}

/// A writer that keeps its changes in the log, with a third session that only the log holds.
fn writer_with_a_log(path: &Path) -> Db {
    let db = filled(path);
    db.conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    db.conn
        .execute("INSERT INTO session(id, title) VALUES('s3', 'third')", [])
        .unwrap();
    assert!(std::fs::metadata(beside(path, "-wal")).unwrap().len() > 0);
    db
}

fn sessions(db: &Db) -> usize {
    db.all_sessions().unwrap().len()
}

#[test]
fn a_database_that_cannot_be_written_is_read_without_its_log_files() {
    if as_root() {
        return;
    }
    let home = Home::new();
    drop(filled(&home.db()));
    assert_eq!(home.files(), ["eye.db"]);
    let _locked = Locked::new(home.0.path());
    let db = Db::open_reading(&home.db()).unwrap();
    assert!(db.is_read_only());
    assert_eq!(sessions(&db), 2);
    // nothing was made beside it: it was read as a file that cannot change
    assert_eq!(home.files(), ["eye.db"]);
}

#[test]
fn a_database_a_writer_holds_open_is_read_through_its_log() {
    if as_root() {
        return;
    }
    let home = Home::new();
    let _writer = writer_with_a_log(&home.db());
    let _locked = Locked::new(home.0.path());
    let db = Db::open_reading(&home.db()).unwrap();
    assert!(db.is_read_only());
    assert_eq!(sessions(&db), 3);
}

#[test]
fn log_files_left_behind_are_read_with_the_changes_in_them() {
    if as_root() {
        return;
    }
    // what a process that stopped without closing leaves: the three files as they were
    let source = Home::new();
    let _writer = writer_with_a_log(&source.db());
    let home = Home::new();
    for suffix in ["", "-wal", "-shm"] {
        std::fs::copy(beside(&source.db(), suffix), beside(&home.db(), suffix)).unwrap();
    }
    let _locked = Locked::new(home.0.path());
    let db = Db::open_reading(&home.db()).unwrap();
    assert_eq!(sessions(&db), 3, "the third session is in the log only");
}

#[test]
fn changes_waiting_in_a_log_that_cannot_be_read_are_refused_not_skipped() {
    if as_root() {
        return;
    }
    let source = Home::new();
    let _writer = writer_with_a_log(&source.db());
    let home = Home::new();
    for suffix in ["", "-wal"] {
        std::fs::copy(beside(&source.db(), suffix), beside(&home.db(), suffix)).unwrap();
    }
    let _locked = Locked::new(home.0.path());
    let e = Db::open_reading(&home.db()).err().unwrap().to_string();
    assert!(
        e.contains("-wal")
            && e.contains("-shm")
            && e.contains("run eye once where it can write")
            && e.contains("SQLite said"),
        "{e}"
    );
    assert_eq!(home.files(), ["eye.db", "eye.db-wal"]);
}

#[test]
fn a_database_file_that_cannot_be_read_is_said_so_even_when_a_log_waits() {
    if as_root() {
        return;
    }
    let source = Home::new();
    let _writer = writer_with_a_log(&source.db());
    let home = Home::new();
    for suffix in ["", "-wal"] {
        std::fs::copy(beside(&source.db(), suffix), beside(&home.db(), suffix)).unwrap();
    }
    std::fs::set_permissions(home.db(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let e = format!("{:#}", Db::open_reading(&home.db()).err().unwrap());
    assert!(e.contains("Permission denied"), "{e}");
    assert!(!e.contains("-shm"), "{e}");
}

#[test]
fn only_a_failure_to_open_the_log_files_is_blamed_on_them() {
    let failure = |code| {
        anyhow::Error::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ))
    };
    assert!(cannot_open_log(&failure(rusqlite::ffi::SQLITE_CANTOPEN)));
    assert!(cannot_open_log(&failure(rusqlite::ffi::SQLITE_READONLY)));
    assert!(!cannot_open_log(&failure(rusqlite::ffi::SQLITE_NOTADB)));
    assert!(!cannot_open_log(&failure(rusqlite::ffi::SQLITE_BUSY)));
    assert!(!cannot_open_log(&anyhow::anyhow!("something else")));
}

#[test]
fn an_empty_log_and_no_index_file_leave_the_database_readable_as_it_is() {
    if as_root() {
        return;
    }
    let home = Home::new();
    drop(filled(&home.db()));
    std::fs::write(beside(&home.db(), "-wal"), "").unwrap();
    let _locked = Locked::new(home.0.path());
    let db = Db::open_reading(&home.db()).unwrap();
    assert_eq!(sessions(&db), 2);
}

#[test]
fn a_path_with_odd_characters_is_read_as_a_file() {
    if as_root() {
        return;
    }
    let home = Home::new();
    let odd = home.0.path().join("a b#c?d%e");
    std::fs::create_dir(&odd).unwrap();
    drop(filled(&odd.join("eye.db")));
    let _locked = Locked::new(&odd);
    assert_eq!(sessions(&Db::open_reading(&odd.join("eye.db")).unwrap()), 2);
}

#[test]
fn a_database_of_another_schema_is_refused_not_upgraded() {
    if as_root() {
        return;
    }
    for (version, said) in [("17", "older eye"), ("99", "newer eye")] {
        let home = Home::new();
        let db = filled(&home.db());
        db.meta_set("schema_version", version).unwrap();
        drop(db);
        let _locked = Locked::new(home.0.path());
        let e = Db::open_reading(&home.db()).err().unwrap().to_string();
        assert!(e.contains(said) && e.contains(version), "{e}");
    }
    // a file that was never set up
    let home = Home::new();
    std::fs::write(home.db(), "").unwrap();
    let _locked = Locked::new(home.0.path());
    let e = Db::open_reading(&home.db()).err().unwrap().to_string();
    assert!(e.contains("not set up"), "{e}");
}

#[test]
fn a_write_to_a_read_only_database_says_so_plainly() {
    if as_root() {
        return;
    }
    let home = Home::new();
    drop(filled(&home.db()));
    let _locked = Locked::new(home.0.path());
    let db = Db::open_reading(&home.db()).unwrap();
    let e = db.set_hidden("s1", Some(true)).unwrap_err();
    let said = explain(&e);
    assert!(said.contains("eye.db is read-only here"), "{said}");
    assert!(!said.contains("readonly"), "{said}");
    // another error is printed as it is
    assert_eq!(
        explain(&anyhow::anyhow!("boom").context("while")),
        "while: boom"
    );
}

#[test]
fn a_writable_database_is_opened_for_writing_and_upgraded_as_before() {
    let home = Home::new();
    let db = filled(&home.db());
    db.meta_set("schema_version", "17").unwrap();
    drop(db);
    let db = Db::open_reading(&home.db()).unwrap();
    assert!(!db.is_read_only());
    assert_eq!(
        db.meta_get("schema_version").unwrap().as_deref(),
        Some("18")
    );
    db.set_hidden("s1", Some(true)).unwrap();
}

#[test]
fn a_missing_database_is_made_by_a_command_that_reads() {
    let home = Home::new();
    let db = Db::open_reading(&home.db()).unwrap();
    assert!(!db.is_read_only() && home.db().is_file());
}

#[test]
fn the_error_for_a_file_that_cannot_be_written_does_not_say_creating() {
    if as_root() {
        return;
    }
    let home = Home::new();
    drop(filled(&home.db()));
    let _locked = Locked::new(home.0.path());
    let e = format!("{:#}", Db::open(&home.db()).err().unwrap());
    assert!(e.contains("cannot be opened for writing"), "{e}");
    assert!(!e.contains("creating"), "{e}");
    // a file that does not exist is still said to be created
    let missing = format!(
        "{:#}",
        Db::open(&home.0.path().join("new.db")).err().unwrap()
    );
    assert!(missing.contains("creating"), "{missing}");
}
