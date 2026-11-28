//! A listing that fails (a folder that cannot be read, a database that is busy) is not "no sessions":
//! the scan must not mark what it did not see as deleted, and a session that comes back is read
//! again even though its file looks the way it did at the last read.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/four_agents.rs"]
mod four_agents;

use agent_eye::agents;
use agent_eye::archive;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use four_agents::{Homes, AGY_A, AGY_B, OC_A, OC_B};
use rusqlite::Connection;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const CLAUDE_A: &str = "a0000001-0000-4000-8000-000000000001";
const CLAUDE_B: &str = "b0000002-0000-4000-8000-000000000002";
const CODEX_A: &str = "019a6a00-1111-7aaa-8bbb-000000000001";

/// Root reads any folder, so a test that makes one unreadable has nothing to test.
fn as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

struct World {
    _tmp: tempfile::TempDir,
    homes: Homes,
    paths: Paths,
    db: Db,
}

fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let homes = four_agents::homes(tmp.path());
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    paths.ensure_home().unwrap();
    let db = Db::open(&paths.db_path()).unwrap();
    let mut w = World {
        _tmp: tmp,
        homes,
        paths,
        db,
    };
    w.scan();
    w
}

impl World {
    fn scan(&mut self) {
        scan(&mut self.db, &self.homes.cfg, false).unwrap();
    }

    fn state(&self, id: &str) -> String {
        self.db
            .conn
            .query_row(
                "SELECT transcript_state FROM session WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn assert_present(&self, ids: &[&str], why: &str) {
        for id in ids {
            assert_eq!(self.state(id), "present", "{id}: {why}");
        }
    }
}

/// `dir` stops being a folder that can be listed (it becomes a file, so reading it fails with
/// something other than "not found"); the returned path puts it back.
fn break_dir(dir: &Path) -> PathBuf {
    let away = dir.with_extension("away");
    std::fs::rename(dir, &away).unwrap();
    std::fs::write(dir, "not a folder").unwrap();
    away
}

fn mend_dir(dir: &Path, away: &Path) {
    std::fs::remove_file(dir).unwrap();
    std::fs::rename(away, dir).unwrap();
}

/// The listing of `dir` fails: the sessions stay present, and stay so when it is back.
fn a_failed_listing_keeps_the_sessions(dir: impl Fn(&Homes) -> PathBuf, ids: &[&str]) {
    let mut w = world();
    w.assert_present(ids, "indexed");
    let dir = dir(&w.homes);
    let away = break_dir(&dir);
    w.scan();
    w.assert_present(ids, "the listing failed, nothing is known to be gone");
    mend_dir(&dir, &away);
    w.scan();
    w.assert_present(ids, "the folder is back");
}

#[test]
fn claude_keeps_its_sessions_when_the_projects_folder_cannot_be_listed() {
    a_failed_listing_keeps_the_sessions(|h| h.claude.join("projects"), &[CLAUDE_A, CLAUDE_B]);
}

/// The Claude fixture's folder of the project that holds `id`.
fn project_of(claude: &Path, id: &str) -> PathBuf {
    std::fs::read_dir(claude.join("projects"))
        .unwrap()
        .flatten()
        .map(|p| p.path())
        .find(|p| p.join(format!("{id}.jsonl")).exists())
        .unwrap()
}

#[test]
fn claude_keeps_its_sessions_when_one_project_folder_cannot_be_listed() {
    if as_root() {
        return;
    }
    let mut w = world();
    let project = project_of(&w.homes.claude, CLAUDE_A);
    std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o000)).unwrap();
    w.scan();
    std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o755)).unwrap();
    w.assert_present(&[CLAUDE_A], "one project folder failed");
}

#[test]
fn codex_keeps_its_sessions_when_the_sessions_folder_cannot_be_listed() {
    a_failed_listing_keeps_the_sessions(|h| h.codex.join("sessions"), &[CODEX_A]);
}

#[test]
fn agy_keeps_its_sessions_when_the_conversations_folder_cannot_be_listed() {
    a_failed_listing_keeps_the_sessions(|h| h.agy.join("conversations"), &[AGY_A, AGY_B]);
}

#[test]
fn opencode_keeps_its_sessions_when_its_database_is_busy() {
    let mut w = world();
    w.assert_present(&[OC_A, OC_B], "indexed");
    let holder = Connection::open(w.homes.opencode.join("opencode.db")).unwrap();
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    w.scan();
    w.assert_present(
        &[OC_A, OC_B],
        "the database was busy, nothing is known to be gone",
    );
    holder.execute_batch("COMMIT").unwrap();
    w.scan();
    w.assert_present(&[OC_A, OC_B], "the database is free again");
}

#[test]
fn a_folder_that_is_not_there_is_an_empty_listing_and_its_sessions_are_gone() {
    let mut w = world();
    std::fs::remove_dir_all(w.homes.claude.join("projects")).unwrap();
    w.scan();
    assert_eq!(w.state(CLAUDE_A), "deleted");
}

#[test]
fn the_adapters_say_whether_a_listing_is_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let Homes { cfg, claude, .. } = four_agents::homes(tmp.path());
    let claude_agent = |cfg: &Config| agents::enabled(cfg).get("claude").unwrap().clone();
    assert!(claude_agent(&cfg).transcripts().complete);
    let away = break_dir(&claude.join("projects"));
    assert!(!claude_agent(&cfg).transcripts().complete);
    mend_dir(&claude.join("projects"), &away);
    std::fs::remove_dir_all(claude.join("projects")).unwrap();
    let listing = claude_agent(&cfg).transcripts();
    assert!(listing.complete && listing.files.is_empty());
}

/// A transcript that went away and came back is read again: its stamp is the one of the last read,
/// and the database still says it was gone.
fn a_transcript_that_comes_back_is_present_again(archived: bool) {
    let mut w = world();
    let file = project_of(&w.homes.claude, CLAUDE_A).join(format!("{CLAUDE_A}.jsonl"));
    if archived {
        let agents = agents::enabled(&w.homes.cfg);
        archive::archive_all(&w.db, &w.paths, &agents).unwrap();
    }
    let away = file.with_extension("away");
    std::fs::rename(&file, &away).unwrap();
    w.scan();
    assert_eq!(
        w.state(CLAUDE_A),
        if archived { "archived" } else { "deleted" }
    );
    std::fs::rename(&away, &file).unwrap();
    w.scan();
    assert_eq!(w.state(CLAUDE_A), "present");
}

#[test]
fn a_deleted_transcript_that_comes_back_is_present_again() {
    a_transcript_that_comes_back_is_present_again(false);
}

#[test]
fn an_archived_transcript_that_comes_back_is_present_again() {
    a_transcript_that_comes_back_is_present_again(true);
}
