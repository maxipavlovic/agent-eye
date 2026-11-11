//! Starting an idea whose folder does not exist.

use super::*;
use crate::folder::FolderMissing;

struct Setup {
    tmp: tempfile::TempDir,
    cfg: Config,
    db: Db,
    paths: Paths,
}

/// An idea in `<tmp>/home/<sub>`, where `<tmp>/home` is the folder eye may create inside.
fn setup(sub: &str) -> (Setup, i64) {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let mut cfg = Config::with_claude_dir(&tmp.path().join("c"));
    cfg.folder_root = Some(home.clone());
    let db = Db::open_memory().unwrap();
    let folder = home.join(sub).to_string_lossy().into_owned();
    let i = add(
        &db,
        &NewIdea {
            title: "Try it",
            folder: Some(&folder),
            ..NewIdea::default()
        },
    )
    .unwrap();
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    (
        Setup {
            tmp,
            cfg,
            db,
            paths,
        },
        i.id,
    )
}

fn dry(create_folder: bool) -> StartOpts {
    StartOpts {
        dry_run: true,
        create_folder,
        ..StartOpts::default()
    }
}

#[test]
fn a_missing_folder_is_refused_unless_asked_for() {
    let (s, id) = setup("new/place");
    let err = start(&s.db, &s.cfg, &s.paths, id, &dry(false)).unwrap_err();
    assert!(err.downcast_ref::<FolderMissing>().is_some(), "{err:#}");
    let err = start(&s.db, &s.cfg, &s.paths, id, &StartOpts::default()).unwrap_err();
    assert!(err.downcast_ref::<FolderMissing>().is_some(), "{err:#}");
    assert!(get(&s.db, id).unwrap().unwrap().session_id.is_none());
}

#[test]
fn a_dry_run_says_it_would_create_the_folder_and_creates_nothing() {
    let (s, id) = setup("new/place");
    let r = start(&s.db, &s.cfg, &s.paths, id, &dry(true)).unwrap();
    assert!(r.dry_run && r.creates_folder);
    assert!(r.command.starts_with("cd '"), "{}", r.command);
    assert!(!s.tmp.path().join("home/new").exists());
}

#[test]
fn an_existing_folder_is_not_created_again() {
    let (s, id) = setup("there");
    std::fs::create_dir(s.tmp.path().join("home/there")).unwrap();
    for create in [false, true] {
        let r = start(&s.db, &s.cfg, &s.paths, id, &dry(create)).unwrap();
        assert!(!r.creates_folder);
    }
}

#[test]
fn a_folder_outside_home_is_refused_even_when_asked() {
    let (mut s, id) = setup("x");
    let other = tempfile::tempdir().unwrap();
    let dir = other.path().join("new").to_string_lossy().into_owned();
    edit(
        &s.db,
        id,
        &Edit {
            folder: Some(dir),
            ..Edit::default()
        },
    )
    .unwrap();
    let err = start(&s.db, &s.cfg, &s.paths, id, &dry(true)).unwrap_err();
    assert!(
        format!("{err:#}").contains("inside your home folder"),
        "{err:#}"
    );
    assert!(err.downcast_ref::<FolderMissing>().is_none());
    s.cfg.folder_root = None;
    assert!(check_folder(&s.cfg, "/nonexistent-eye-test/x", true).is_err());
    assert!(!other.path().join("new").exists());
}

#[test]
fn a_file_where_the_folder_should_be_is_refused() {
    let (s, id) = setup("afile");
    std::fs::write(s.tmp.path().join("home/afile"), "x").unwrap();
    let err = start(&s.db, &s.cfg, &s.paths, id, &dry(true)).unwrap_err();
    assert_eq!(format!("{err:#}"), "Not a folder");
}

#[test]
fn checking_a_folder_says_whether_it_needs_making() {
    let (s, _) = setup("x");
    let root = s.tmp.path().join("home");
    let here = root.to_string_lossy().into_owned();
    assert!(!check_folder(&s.cfg, &here, false).unwrap());
    let new = root.join("n").to_string_lossy().into_owned();
    assert!(check_folder(&s.cfg, &new, true).unwrap());
    assert!(check_folder(&s.cfg, &new, false).is_err());
}
