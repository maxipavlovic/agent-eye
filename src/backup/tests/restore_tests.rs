use super::*;
use crate::platform::test_support::{self, FAST};

fn snapshot_of(h: &Home, rows_in: i64, reason: &Reason) -> Entry {
    let db = db_with(&h.paths, rows_in);
    let s = snapshot(&db.conn, &dir(&h.paths), reason).unwrap();
    Entry::from_path(&s.path).unwrap()
}

#[test]
fn a_restore_refuses_a_damaged_snapshot_and_a_newer_one() {
    let h = home();
    let d = dir(&h.paths);
    touch(&d, "eye-20261101-030000-manual.db.zst");
    let e = find(&d, "latest").unwrap();
    assert!(prepare(&h.paths, e).is_err(), "not zstd at all");
    // a sound database that says it is from the future
    let h = home();
    let db = db_with(&h.paths, 1);
    db.meta_set("schema_version", "99").unwrap();
    let s = snapshot(&db.conn, &dir(&h.paths), &Reason::Manual).unwrap();
    let err = prepare(&h.paths, Entry::from_path(&s.path).unwrap()).unwrap_err();
    assert!(err.to_string().contains("newer eye"), "{err:#}");
    // zstd of something that is not a database
    let h = home();
    let d = dir(&h.paths);
    private_dir(&d).unwrap();
    std::fs::write(h.paths.home.join("junk"), vec![7u8; 4096]).unwrap();
    crate::archive::compress_file(
        &h.paths.home.join("junk"),
        &d.join("eye-20261101-030000-manual.db.zst"),
    )
    .unwrap();
    assert!(prepare(&h.paths, find(&d, "latest").unwrap()).is_err());
    let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().collect();
    assert_eq!(left.len(), 1, "no temp file stays after a refusal");
}

#[test]
fn a_snapshot_is_found_by_name_with_or_without_its_suffix_and_by_latest() {
    let h = home();
    let d = dir(&h.paths);
    touch(&d, "eye-20261101-030000-daily.db.zst");
    touch(&d, "eye-20261102-030000-manual.db.zst");
    assert_eq!(
        find(&d, "latest").unwrap().name,
        "eye-20261102-030000-manual.db.zst"
    );
    assert!(find(&d, "eye-20261101-030000-daily").is_ok());
    assert!(find(&d, "eye-20261101-030000-daily.db.zst").is_ok());
    assert!(find(&d, "nothing").is_err());
    assert!(!is_snapshot(&d, "0123456789abcdef"));
    assert!(
        find(&dir(&home().paths), "latest").is_err(),
        "no folder, no snapshot"
    );
}

#[test]
fn a_restore_replaces_the_database_drops_the_old_wal_and_keeps_a_pre_restore_copy() {
    let h = home();
    let old = snapshot_of(&h, 3, &Reason::Manual);
    // the database moves on, and keeps its newest commits in the WAL
    let db = db_with(&h.paths, 7);
    db.conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    db.conn
        .execute("INSERT INTO probe VALUES(100)", [])
        .unwrap();
    let wal = h.paths.home.join("eye.db-wal");
    assert!(wal.metadata().unwrap().len() > 0);
    let prepared = prepare(&h.paths, old).unwrap();
    let done = apply(&opts(&h, true), &Config::default(), &prepared).unwrap();
    assert!(!wal.exists(), "a stale WAL on a restored file corrupts it");
    assert!(!h.paths.home.join("eye.db-shm").exists());
    assert_eq!(rows(&h.paths.db_path()), 3);
    let kept = done.pre_restore.unwrap();
    assert!(kept.to_string_lossy().ends_with("-pre-restore.db.zst"));
    let out = h.paths.home.join("kept.db");
    unpack_to(&kept, &out);
    assert_eq!(
        rows(&out),
        8,
        "the copy has the commits that were only in the WAL"
    );
    assert!(!done.daemon_restarted);
    let leftovers = std::fs::read_dir(dir(&h.paths))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn a_restore_with_no_database_to_keep_just_puts_the_snapshot_in_place() {
    let h = home();
    let old = snapshot_of(&h, 2, &Reason::Daily);
    std::fs::remove_file(h.paths.db_path()).unwrap();
    for ext in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(h.paths.home.join(format!("eye.db{ext}")));
    }
    let prepared = prepare(&h.paths, old).unwrap();
    let done = apply(&opts(&h, true), &Config::default(), &prepared).unwrap();
    assert!(done.pre_restore.is_none());
    assert_eq!(rows(&h.paths.db_path()), 2);
}

#[test]
fn a_restore_stops_the_daemon_through_the_service_and_starts_it_again() {
    let h = home();
    let old = snapshot_of(&h, 2, &Reason::Manual);
    let progs = h.paths.home.join("programs");
    std::fs::create_dir_all(&progs).unwrap();
    let mut o = opts(&h, false);
    o.programs = Some(progs.clone().into());
    o.timing = FAST;
    std::fs::create_dir_all(&o.service_dir).unwrap();
    std::fs::write(
        o.service_dir.join(o.manager.file_name()),
        crate::install::unit_text(&o, &Default::default()),
    )
    .unwrap();
    match o.manager {
        Manager::Launchd => {
            test_support::launchd(&progs);
            test_support::set(&progs, "loaded", "");
        }
        Manager::Systemd => {
            test_support::systemd(&progs);
            test_support::set(&progs, "active", "");
        }
    }
    // a daemon that is alive but holds no lock: this process stands in for it
    std::fs::write(
        h.paths.home.join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    let prepared = prepare(&h.paths, old).unwrap();
    let done = apply(&o, &Config::default(), &prepared).unwrap();
    assert!(done.daemon_restarted, "{:?}", done.restart_failed);
    let tool = if o.manager == Manager::Launchd {
        "launchctl"
    } else {
        "systemctl"
    };
    let calls = test_support::calls(&progs, tool);
    let stop = if o.manager == Manager::Launchd {
        "bootout"
    } else {
        "disable"
    };
    let start = if o.manager == Manager::Launchd {
        "bootstrap"
    } else {
        "enable"
    };
    let at = |w: &str| calls.iter().position(|c| c == w);
    assert!(at(stop).unwrap() < at(start).unwrap(), "{calls:?}");
}

#[test]
fn a_daemon_that_does_not_run_from_the_service_stops_the_restore_before_anything_changes() {
    let h = home();
    let old = snapshot_of(&h, 2, &Reason::Manual);
    let db = db_with(&h.paths, 9);
    std::fs::write(
        h.paths.home.join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    let prepared = prepare(&h.paths, old).unwrap();
    let err = apply(&opts(&h, true), &Config::default(), &prepared).unwrap_err();
    assert!(
        err.to_string().contains("does not run from the service"),
        "{err:#}"
    );
    assert_eq!(rows(&h.paths.db_path()), 9);
    drop(db);
}
