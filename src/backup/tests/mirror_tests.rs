use super::*;
use std::os::unix::fs::PermissionsExt;

fn mirror_cfg(m: &Path) -> BackupCfg {
    BackupCfg {
        mirror: m.to_string_lossy().into_owned(),
        ..BackupCfg::default()
    }
}

#[test]
fn a_mirror_that_is_not_there_is_skipped_never_created_and_noted() {
    let h = home();
    let db = db_with(&h.paths, 1);
    let gone = h.paths.home.join("Volumes/Disk/eye");
    let cfg = mirror_cfg(&gone);
    let s = take(&db.conn, &h.paths, &cfg, &Reason::Manual).unwrap();
    assert!(s.path.exists(), "the local snapshot is made anyway");
    assert!(!gone.exists() && !gone.parent().unwrap().exists());
    let Some(mirror::State::Failed(_, why)) = mirror::state(&h.paths) else {
        panic!("not noted");
    };
    assert!(why.contains("does not exist"), "{why}");
}

#[test]
fn a_mirror_gets_each_snapshot_and_the_same_rotation_for_eye_files_only() {
    let h = home();
    let db = db_with(&h.paths, 1);
    let m = h.paths.home.join("disk");
    std::fs::create_dir_all(&m).unwrap();
    touch(&m, "photos.txt");
    for day in 1..=5 {
        touch(&m, &format!("eye-202611{day:02}-030000Z-manual.db.zst"));
    }
    for _ in 0..2 {
        take(&db.conn, &h.paths, &mirror_cfg(&m), &Reason::Manual).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    assert!(m.join("photos.txt").exists());
    assert_eq!(
        list(&m).len(),
        5,
        "manual snapshots are capped at 5 there too"
    );
    let newest = list(&dir(&h.paths))[0].name.clone();
    assert!(m.join(&newest).exists());
    assert_eq!(
        std::fs::metadata(m.join(&newest))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(matches!(
        mirror::state(&h.paths),
        Some(mirror::State::Ok(_))
    ));
}

#[test]
fn the_archive_follows_the_mirror_only_where_a_file_changed_and_nothing_is_deleted() {
    let h = home();
    let db = db_with(&h.paths, 1);
    let arch = h.paths.home.join("archive/-proj");
    std::fs::create_dir_all(&arch).unwrap();
    std::fs::write(arch.join("a.jsonl.zst"), b"first").unwrap();
    std::fs::write(arch.join("b.jsonl.zst"), b"second").unwrap();
    let m = h.paths.home.join("disk");
    std::fs::create_dir_all(m.join("archive")).unwrap();
    std::fs::write(m.join("archive/keep-me.txt"), b"mine").unwrap();
    let mut cfg = mirror_cfg(&m);
    cfg.mirror_archive = true;
    take(&db.conn, &h.paths, &cfg, &Reason::Manual).unwrap();
    let copy = |n: &str| m.join("archive/-proj").join(n);
    assert_eq!(std::fs::read(copy("a.jsonl.zst")).unwrap(), b"first");
    assert_eq!(std::fs::read(copy("b.jsonl.zst")).unwrap(), b"second");
    // tamper with a copy without changing its size or time: an unchanged file is not copied again
    let mtime = std::fs::metadata(copy("a.jsonl.zst"))
        .unwrap()
        .modified()
        .unwrap();
    std::fs::write(copy("a.jsonl.zst"), b"FIRST").unwrap();
    std::fs::File::options()
        .write(true)
        .open(copy("a.jsonl.zst"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    std::fs::write(arch.join("b.jsonl.zst"), b"second, longer").unwrap();
    std::fs::remove_file(arch.join("a.jsonl.zst")).unwrap();
    std::fs::write(arch.join("a.jsonl.zst"), b"first").unwrap();
    std::fs::File::options()
        .write(true)
        .open(arch.join("a.jsonl.zst"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    take(&db.conn, &h.paths, &cfg, &Reason::Manual).unwrap();
    assert_eq!(std::fs::read(copy("a.jsonl.zst")).unwrap(), b"FIRST");
    assert_eq!(
        std::fs::read(copy("b.jsonl.zst")).unwrap(),
        b"second, longer"
    );
    std::fs::remove_file(arch.join("b.jsonl.zst")).unwrap();
    take(&db.conn, &h.paths, &cfg, &Reason::Manual).unwrap();
    assert!(
        copy("b.jsonl.zst").exists(),
        "nothing is deleted in the mirror"
    );
    assert!(m.join("archive/keep-me.txt").exists());
}

fn lines(h: &Home, cfg: &Config) -> Vec<(bool, String)> {
    checks(&h.paths, cfg, crate::util::now_ms())
        .into_iter()
        .map(|c| (c.ok, c.msg))
        .collect()
}

#[test]
fn doctor_says_what_the_snapshots_and_the_mirror_are_doing() {
    let h = home();
    let cfg = Config::default();
    let l = lines(&h, &cfg);
    assert_eq!(
        l[0],
        (true, "backups: none yet".to_string()),
        "no database yet"
    );
    assert!(l.contains(&(true, "backup mirror: off".to_string())));
    let db = db_with(&h.paths, 1);
    let l = lines(&h, &cfg);
    assert!(!l[0].0 && l[0].1.contains("none yet"), "{l:?}");
    assert!(l.contains(&(true, "database check: ok".to_string())));
    take(&db.conn, &h.paths, &cfg.backup, &Reason::Daily).unwrap();
    let l = lines(&h, &cfg);
    assert!(
        l[0].0 && l[0].1.starts_with("backups: 1 snapshots") && l[0].1.contains("(daily)"),
        "{l:?}"
    );
    // newest older than 3 days
    let d = dir(&h.paths);
    for e in list(&d) {
        std::fs::remove_file(e.path).unwrap();
    }
    touch(&d, "eye-20200101-030000Z-daily.db.zst");
    let l = lines(&h, &cfg);
    assert!(!l[0].0 && l[0].1.contains("is the daemon running"), "{l:?}");
    let mut off = cfg.clone();
    off.backup.enabled = false;
    let l = lines(&h, &off);
    assert!(
        l[0].0 && l[0].1.contains("automatic snapshots are off"),
        "{l:?}"
    );
}

#[test]
fn doctor_reports_a_missing_or_failing_mirror() {
    let h = home();
    let mut cfg = Config::default();
    cfg.backup.mirror = h.paths.home.join("disk").to_string_lossy().into_owned();
    let mirror_line = |h: &Home, cfg: &Config| {
        lines(h, cfg)
            .into_iter()
            .find(|l| l.1.starts_with("backup mirror"))
            .unwrap()
    };
    let l = mirror_line(&h, &cfg);
    assert!(!l.0 && l.1.contains("is missing"), "{l:?}");
    std::fs::create_dir_all(h.paths.home.join("disk")).unwrap();
    let l = mirror_line(&h, &cfg);
    assert!(l.0 && l.1.contains("no copy made yet"), "{l:?}");
    mirror::sync(&h.paths, &cfg.backup, &h.paths.home.join("nope"));
    let l = mirror_line(&h, &cfg);
    assert!(!l.0 && l.1.contains("failed"), "{l:?}");
    let db = db_with(&h.paths, 1);
    take(&db.conn, &h.paths, &cfg.backup, &Reason::Manual).unwrap();
    let l = mirror_line(&h, &cfg);
    assert!(l.0 && l.1.contains("last copy"), "{l:?}");
}

#[test]
fn doctor_flags_a_database_that_fails_its_check() {
    let h = home();
    std::fs::write(h.paths.db_path(), vec![9u8; 8192]).unwrap();
    let l = lines(&h, &Config::default());
    assert!(
        l.iter().all(|x| !x.1.starts_with("database check: ok")),
        "{l:?}"
    );
}
