use super::*;
use crate::config::BackupCfg;
use crate::db::{Db, SCHEMA_VERSION};
use crate::install::Opts;
use crate::platform::service::{Manager, Timing};
use std::os::unix::fs::PermissionsExt;

mod mirror_tests;
mod restore_tests;

/// An eye home in a temp folder.
pub(super) struct Home {
    _t: tempfile::TempDir,
    pub paths: Paths,
}

pub(super) fn home() -> Home {
    let t = tempfile::tempdir().unwrap();
    let paths = Paths {
        home: t.path().join("eye"),
    };
    paths.ensure_home().unwrap();
    Home { _t: t, paths }
}

/// A database with `n` rows in a table of its own, at the current schema.
pub(super) fn db_with(paths: &Paths, n: i64) -> Db {
    let db = Db::open(&paths.db_path()).unwrap();
    db.conn
        .execute_batch("CREATE TABLE IF NOT EXISTS probe(n INTEGER PRIMARY KEY)")
        .unwrap();
    for i in 1..=n {
        db.conn
            .execute("INSERT OR IGNORE INTO probe VALUES(?1)", [i])
            .unwrap();
    }
    db
}

pub(super) fn rows(db: &Path) -> i64 {
    Connection::open(db)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM probe", [], |r| r.get(0))
        .unwrap()
}

pub(super) fn touch(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), b"x").unwrap();
}

pub(super) fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = list(dir)
        .into_iter()
        .map(|e| e.name.trim_end_matches(".db.zst").to_string())
        .collect();
    v.sort();
    v
}

pub(super) fn opts(h: &Home, no_service: bool) -> Opts {
    Opts {
        eye_home: h.paths.home.clone(),
        bin_dir: h.paths.home.join("bin"),
        manager: Manager::current(),
        service_dir: h.paths.home.join("agents"),
        exe: std::env::current_exe().unwrap(),
        dry_run: false,
        no_service,
        purge: false,
        board: false,
        programs: None,
        timing: Timing::REAL,
    }
}

pub(super) fn unpack_to(path: &Path, to: &Path) {
    crate::archive::decompress_file(path, to).unwrap();
}

#[test]
fn a_snapshot_is_a_sound_database_while_another_connection_writes() {
    let h = home();
    let db = db_with(&h.paths, 0);
    let path = h.paths.db_path();
    let writer = std::thread::spawn(move || {
        let w = Connection::open(path).unwrap();
        w.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        for i in 1..=2000 {
            w.execute("INSERT INTO probe VALUES(?1)", [i]).unwrap();
        }
    });
    let mut snaps = vec![];
    while !writer.is_finished() {
        snaps.push(snapshot(&db.conn, &dir(&h.paths), &Reason::Manual).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    writer.join().unwrap();
    snaps.push(snapshot(&db.conn, &dir(&h.paths), &Reason::Daily).unwrap());
    assert!(!snaps.is_empty());
    for (i, s) in snaps.iter().enumerate() {
        let out = h.paths.home.join(format!("check{i}.db"));
        unpack_to(&s.path, &out);
        let c = Connection::open(&out).unwrap();
        assert_eq!(restore::quick_check(&c), None);
        let (count, max): (i64, i64) = c
            .query_row("SELECT COUNT(*), COALESCE(MAX(n), 0) FROM probe", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(count, max, "a snapshot holds whole commits only");
    }
    let last = snaps.last().unwrap();
    let out = h.paths.home.join("last.db");
    unpack_to(&last.path, &out);
    assert_eq!(rows(&out), 2000);
}

#[test]
fn snapshots_are_named_private_and_leave_no_temp_files() {
    let h = home();
    let db = db_with(&h.paths, 3);
    let s = snapshot(&db.conn, &dir(&h.paths), &Reason::PreUpgrade(10)).unwrap();
    let name = s.path.file_name().unwrap().to_str().unwrap();
    assert!(
        name.starts_with("eye-") && name.ends_with("-pre-v10.db.zst"),
        "{name}"
    );
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&s.path), 0o600);
    assert_eq!(mode(&dir(&h.paths)), 0o700);
    let entries: Vec<_> = std::fs::read_dir(dir(&h.paths)).unwrap().collect();
    assert_eq!(entries.len(), 1, "only the snapshot is left");
    let e = &list(&dir(&h.paths))[0];
    assert_eq!(e.reason, Reason::PreUpgrade(10));
    assert_eq!(e.bytes, s.bytes);
}

#[test]
fn a_failed_snapshot_leaves_nothing_behind() {
    let h = home();
    let db = db_with(&h.paths, 1);
    // VACUUM cannot run inside a transaction
    db.conn.execute_batch("BEGIN").unwrap();
    assert!(snapshot(&db.conn, &dir(&h.paths), &Reason::Manual).is_err());
    assert_eq!(std::fs::read_dir(dir(&h.paths)).unwrap().count(), 0);
}

#[test]
fn only_files_named_as_eye_names_them_are_snapshots() {
    let h = home();
    let d = dir(&h.paths);
    for n in [
        "eye-20261101-100000-daily.db.zst",
        "eye-20261101-100000-other.db.zst",
        "eye-2026-daily.db.zst",
        "notes.txt",
        "eye-20261101-100000-daily.db",
        "mirror.state",
    ] {
        touch(&d, n);
    }
    assert_eq!(names(&d), ["eye-20261101-100000-daily"]);
}

fn cfg(daily: usize, weekly: usize) -> BackupCfg {
    BackupCfg {
        keep_daily: daily,
        keep_weekly: weekly,
        ..BackupCfg::default()
    }
}

#[test]
fn rotation_keeps_the_newest_dailies_one_per_week_and_the_last_of_each_other_kind() {
    let h = home();
    let d = dir(&h.paths);
    // 21 dailies, 1 Nov to 21 Nov 2026 (a Sunday starts the ISO week of 2 Nov)
    for day in 1..=21 {
        touch(&d, &format!("eye-202611{day:02}-030000-daily.db.zst"));
    }
    for v in 5..=10 {
        touch(&d, &format!("eye-202610{v:02}-030000-pre-v{v}.db.zst"));
        touch(&d, &format!("eye-202610{v:02}-040000-pre-restore.db.zst"));
    }
    for m in 1..=8 {
        touch(&d, &format!("eye-202609{m:02}-030000-manual.db.zst"));
    }
    touch(&d, "mirror.state");
    touch(&d, "my-notes.txt");
    rotate(&d, &cfg(3, 2));
    let left = names(&d);
    let of = |w: &str| left.iter().filter(|n| n.ends_with(w)).count();
    assert_eq!(of("-manual"), 5);
    assert_eq!(of("-restore"), 3);
    assert!(left.contains(&"eye-20261008-030000-pre-v8".to_string()));
    assert!(!left.contains(&"eye-20261007-030000-pre-v7".to_string()));
    assert_eq!(left.iter().filter(|n| n.contains("-pre-v")).count(), 3);
    let dailies: Vec<&String> = left.iter().filter(|n| n.ends_with("-daily")).collect();
    // newest 3: 19, 20, 21 Nov; weeks: 16-22 Nov (newest is the 21st) and 9-15 Nov (the 15th)
    assert_eq!(
        dailies.iter().map(|n| &n[4..12]).collect::<Vec<_>>(),
        ["20261115", "20261119", "20261120", "20261121"]
    );
    assert!(d.join("mirror.state").exists() && d.join("my-notes.txt").exists());
}

#[test]
fn rotation_with_nothing_to_keep_or_no_folder_does_nothing_wrong() {
    let h = home();
    rotate(&dir(&h.paths), &BackupCfg::default());
    let d = dir(&h.paths);
    touch(&d, "eye-20261101-030000-daily.db.zst");
    rotate(&d, &cfg(0, 0));
    assert!(names(&d).is_empty());
}

#[test]
fn old_temp_files_are_cleaned_and_young_ones_are_not() {
    let h = home();
    let d = dir(&h.paths);
    touch(&d, ".eye-old.db.tmp");
    touch(&d, ".eye-young.db.tmp");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
    std::fs::File::options()
        .write(true)
        .open(d.join(".eye-old.db.tmp"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    rotate(&d, &BackupCfg::default());
    assert!(!d.join(".eye-old.db.tmp").exists());
    assert!(d.join(".eye-young.db.tmp").exists());
}

fn old_database(h: &Home, version: &str) {
    let db = db_with(&h.paths, 4);
    db.meta_set("schema_version", version).unwrap();
}

fn pre_upgrade_names(h: &Home) -> Vec<String> {
    names(&dir(&h.paths))
        .into_iter()
        .filter(|n| n.contains("-pre-v"))
        .collect()
}

#[test]
fn an_older_database_is_snapshotted_once_before_it_is_migrated() {
    let h = home();
    old_database(&h, "8");
    let db = Db::open(&h.paths.db_path()).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
    let snaps = pre_upgrade_names(&h);
    assert_eq!(snaps.len(), 1, "{snaps:?}");
    assert!(snaps[0].ends_with(&format!("-pre-v{SCHEMA_VERSION}")));
    // the copy is the database as it was
    let out = h.paths.home.join("was.db");
    unpack_to(&list(&dir(&h.paths))[0].path, &out);
    let was = Connection::open(&out).unwrap();
    assert_eq!(restore::schema_of(&was).unwrap(), 8);
    assert_eq!(rows(&out), 4);
    drop(db);
    Db::open(&h.paths.db_path()).unwrap();
    assert_eq!(
        pre_upgrade_names(&h).len(),
        1,
        "an up to date database needs none"
    );
}

#[test]
fn a_new_or_empty_database_gets_no_snapshot() {
    let h = home();
    Db::open(&h.paths.db_path()).unwrap();
    assert!(list(&dir(&h.paths)).is_empty());
    let empty = Connection::open_in_memory().unwrap();
    assert!(!needs_upgrade(&empty, SCHEMA_VERSION));
    let c = Connection::open(h.paths.home.join("old.db")).unwrap();
    c.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES('schema_version','3')")
        .unwrap();
    assert!(needs_upgrade(&c, 4));
    assert!(!needs_upgrade(&c, 3));
}

#[test]
fn a_database_from_before_versions_with_sessions_counts_as_old() {
    let h = home();
    let c = Connection::open(h.paths.db_path()).unwrap();
    c.execute_batch("CREATE TABLE session(id TEXT PRIMARY KEY); INSERT INTO session VALUES('a')")
        .unwrap();
    assert!(needs_upgrade(&c, SCHEMA_VERSION));
}

#[test]
fn automatic_snapshots_can_be_turned_off_for_upgrades_too() {
    let h = home();
    std::fs::write(h.paths.config_path(), "[backup]\nenabled = false\n").unwrap();
    old_database(&h, "8");
    Db::open(&h.paths.db_path()).unwrap();
    assert!(list(&dir(&h.paths)).is_empty());
}

#[test]
fn a_snapshot_that_cannot_be_made_does_not_stop_the_upgrade() {
    let h = home();
    old_database(&h, "8");
    // a file where the folder should be
    std::fs::write(dir(&h.paths), "in the way").unwrap();
    let db = Db::open(&h.paths.db_path()).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(SCHEMA_VERSION.to_string())
    );
}

#[test]
fn two_upgrades_at_once_leave_one_sound_snapshot() {
    let h = home();
    old_database(&h, "8");
    let path = h.paths.db_path();
    let runs: Vec<_> = (0..4)
        .map(|_| {
            let p = path.clone();
            std::thread::spawn(move || Db::open(&p).map(drop))
        })
        .collect();
    for r in runs {
        r.join().unwrap().unwrap();
    }
    let all = list(&dir(&h.paths));
    assert!(!all.is_empty());
    assert!(all
        .iter()
        .all(|e| e.reason == Reason::PreUpgrade(SCHEMA_VERSION)));
    let out = h.paths.home.join("was.db");
    unpack_to(&all[0].path, &out);
    assert_eq!(restore::quick_check(&Connection::open(&out).unwrap()), None);
    let temps = std::fs::read_dir(dir(&h.paths))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(temps, 0);
}

#[test]
fn the_daily_snapshot_is_made_when_the_newest_is_over_a_day_old() {
    let h = home();
    let db = db_with(&h.paths, 1);
    let cfg = Config::default();
    let now = crate::util::now_ms();
    assert!(daily_if_due(&db.conn, &h.paths, &cfg, now)
        .unwrap()
        .is_some());
    assert!(daily_if_due(&db.conn, &h.paths, &cfg, now + 3_600_000)
        .unwrap()
        .is_none());
    // a manual one does not count as a daily
    assert_eq!(list(&dir(&h.paths)).len(), 1);
    let later = now + DAILY_EVERY_MS + 60_000;
    assert!(daily_if_due(&db.conn, &h.paths, &cfg, later)
        .unwrap()
        .is_some());
}

#[test]
fn no_daily_snapshot_when_automatic_ones_are_off_but_a_manual_one_works() {
    let h = home();
    let db = db_with(&h.paths, 1);
    let mut cfg = Config::default();
    cfg.backup.enabled = false;
    assert!(
        daily_if_due(&db.conn, &h.paths, &cfg, crate::util::now_ms())
            .unwrap()
            .is_none()
    );
    assert!(list(&dir(&h.paths)).is_empty());
    take(&db.conn, &h.paths, &cfg.backup, &Reason::Manual).unwrap();
    assert_eq!(list(&dir(&h.paths)).len(), 1);
}
