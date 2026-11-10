//! Copies of sessions under `$EYE_HOME/archive/`, and putting them back. Where and when is decided
//! here; how a session is copied and restored is the adapter's call (`Agent::archive` and
//! `Agent::unarchive`). By default that is one zstd file per session.

use crate::agents::{Agent, Agents, LEGACY_ID};
use crate::db::{meta_set, Db};
use crate::model::Session;
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::params;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const LEVEL: i32 = 9;

/// What archiving one session did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Archived {
    /// A new copy was written (bytes on disk).
    Copied(u64),
    Unchanged,
    /// Nothing to copy: no transcript on disk, or the session's agent is not enabled.
    NoTranscript,
}

/// Where the copy of a session lives. Claude Code's copies keep the layout from before eye knew
/// several agents (`archive/<project dir>/<id>.jsonl.zst`); another agent gets its own folder,
/// `archive/@<agent>/...` (no project dir starts with `@`). The file name is the adapter's
/// (`Agent::archive_name`).
pub fn archive_path(paths: &Paths, agent: &dyn Agent, project_dir: &str, id: &str) -> PathBuf {
    let base = paths.home.join("archive");
    let base = match agent.id() {
        LEGACY_ID => base,
        other => base.join(format!("@{other}")),
    };
    base.join(project_dir).join(agent.archive_name(id))
}

/// Compress one file to `dest` with zstd (mode 0600). The default `Agent::archive`, and a building
/// block for adapters that keep a session in several files.
pub fn compress_file(src: &Path, dest: &Path) -> Result<()> {
    let input = std::fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(dest)?;
    zstd::stream::copy_encode(input, &output, LEVEL).context("compressing")?;
    output.sync_all()?;
    Ok(())
}

/// Decompress a file made by [`compress_file`] to `dest`, which must not exist (mode 0600).
pub fn decompress_file(src: &Path, dest: &Path) -> Result<()> {
    let input = std::fs::File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dest)?;
    zstd::stream::copy_decode(input, &output).context("decompressing")?;
    output.sync_all()?;
    Ok(())
}

/// `path` with `suffix` added to its file name: where a copy is built before it is put in place.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Remove a file or a folder and what is in it; nothing happens when it is not there.
fn remove_any(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

/// Bytes on disk of a file, or of every file in a folder.
fn disk_size(path: &Path) -> u64 {
    let Ok(md) = std::fs::metadata(path) else {
        return 0;
    };
    if !md.is_dir() {
        return md.len();
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| disk_size(&e.path()))
        .sum()
}

/// Copy the session `id` when it changed since the last copy.
pub fn archive_session(db: &Db, paths: &Paths, agents: &Agents, id: &str) -> Result<Archived> {
    let s = db.session_by_id(id)?.context("no such session")?;
    let Some(agent) = agents.of_session(&s) else {
        return Ok(Archived::NoTranscript);
    };
    let (Some(src), Some(pd)) = (s.transcript_path.as_deref(), s.project_dir.as_deref()) else {
        return Ok(Archived::NoTranscript);
    };
    let src = Path::new(src);
    if s.transcript_state.as_deref() != Some("present") {
        return Ok(Archived::NoTranscript);
    }
    let Some(stamp) = agent.transcript_stamp(src) else {
        return Ok(Archived::NoTranscript);
    };
    let sig = format!("{}:{}", stamp.size, stamp.mtime);
    let dest = archive_path(paths, agent.as_ref(), pd, id);
    let key = format!("archived_sig:{id}");
    if dest.exists() && db.meta_get(&key)?.as_deref() == Some(sig.as_str()) {
        return Ok(Archived::Unchanged);
    }
    std::fs::create_dir_all(dest.parent().context("archive path has no parent")?)?;
    let tmp = sibling(&dest, ".tmp");
    remove_any(&tmp);
    agent.archive(src, &tmp).inspect_err(|_| remove_any(&tmp))?;
    if dest.is_dir() {
        std::fs::remove_dir_all(&dest)?;
    }
    std::fs::rename(&tmp, &dest)?;
    let size = disk_size(&dest);
    // The stamp was taken before reading: if the session changed meanwhile, the next pass copies again.
    meta_set(&db.conn, &key, &sig)?;
    db.conn.execute(
        "UPDATE session SET archive_path=?2 WHERE id=?1",
        params![id, dest.to_string_lossy()],
    )?;
    Ok(Archived::Copied(size))
}

/// Archive every session that has a transcript on disk.
pub fn archive_all(db: &Db, paths: &Paths, agents: &Agents) -> Result<Vec<(String, Archived)>> {
    let ids: Vec<String> = db
        .all_sessions()?
        .into_iter()
        .filter(|s| s.transcript_state.as_deref() == Some("present"))
        .map(|s| s.id)
        .collect();
    let mut out = vec![];
    for id in ids {
        match archive_session(db, paths, agents, &id) {
            Ok(r) => out.push((id, r)),
            Err(e) => eprintln!("warn: archive {id}: {e:#}"),
        }
    }
    Ok(out)
}

/// The archived copy of a session: the stored path, else where it would be.
fn copy_of(s: &Session, paths: &Paths, agent: &dyn Agent) -> PathBuf {
    match s.archive_path.as_deref() {
        Some(p) => PathBuf::from(p),
        None => archive_path(paths, agent, s.project_dir.as_deref().unwrap_or(""), &s.id),
    }
}

/// Put the archived copy back at the original transcript path, only when that path is missing.
/// Returns the path written.
pub fn restore(db: &Db, paths: &Paths, agents: &Agents, id: &str) -> Result<PathBuf> {
    let s = db.session_by_id(id)?.context("no such session")?;
    let agent = agents
        .of_session(&s)
        .with_context(|| format!("the agent of this session ({}) is not enabled", s.agent))?;
    let dest = PathBuf::from(
        s.transcript_path
            .as_deref()
            .context("the original transcript path is unknown")?,
    );
    anyhow::ensure!(
        agent.owns_transcript(&dest) && !dest.components().any(|c| c.as_os_str() == ".."),
        "{} is not a transcript path of {}",
        dest.display(),
        agent.name()
    );
    anyhow::ensure!(
        !dest.exists(),
        "{} already exists; nothing restored",
        dest.display()
    );
    let src = copy_of(&s, paths, agent.as_ref());
    anyhow::ensure!(src.exists(), "no archived copy at {}", src.display());
    std::fs::create_dir_all(dest.parent().context("no parent dir")?)?;
    let tmp = sibling(&dest, ".restoring");
    remove_any(&tmp);
    agent
        .unarchive(&src, &tmp)
        .inspect_err(|_| remove_any(&tmp))?;
    std::fs::rename(&tmp, &dest)?;
    Ok(dest)
}

/// Put the archived copy of a session's transcript back when its agent has deleted the original, so
/// that resuming works. Nothing happens when the transcript is there or no copy was kept.
pub fn restore_if_missing(db: &Db, paths: &Paths, agents: &Agents, s: &Session) -> Result<()> {
    if let Some(tp) = s.transcript_path.as_deref() {
        if !Path::new(tp).exists() && s.archive_path.is_some() {
            restore(db, paths, agents, &s.id)
                .context("restoring the transcript from eye's archive")?;
        }
    }
    Ok(())
}

/// A session's transcript as a path that exists on disk: the live one, or a temporary copy made
/// from the archive. The copy lives in `$EYE_HOME/tmp` and is removed when this is dropped.
pub struct Readable {
    pub path: PathBuf,
    scratch: Option<PathBuf>,
}

impl Drop for Readable {
    fn drop(&mut self) {
        if let Some(dir) = &self.scratch {
            remove_any(dir);
        }
    }
}

/// Make the transcript of `s` readable: the file (or folder) itself while it is there, else a copy
/// the agent restores from the archive.
pub fn readable(s: &Session, paths: &Paths, agent: &dyn Agent) -> Result<Readable> {
    let live = s.transcript_path.as_deref().map(PathBuf::from);
    if let Some(p) = live.as_ref().filter(|p| p.exists()) {
        return Ok(Readable {
            path: p.clone(),
            scratch: None,
        });
    }
    let copy = copy_of(s, paths, agent);
    anyhow::ensure!(copy.exists(), "no transcript or archived copy to read");
    let dir = paths
        .home
        .join("tmp")
        .join(format!("read-{}", crate::util::new_uuid()?));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let name = live
        .as_ref()
        .and_then(|p| p.file_name())
        .unwrap_or(s.id.as_ref());
    let r = Readable {
        path: dir.join(name),
        scratch: Some(dir),
    };
    agent
        .unarchive(&copy, &r.path)
        .context("reading the archived copy")?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, Paths, Db, PathBuf, Agents) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let db = Db::open(&paths.db_path()).unwrap();
        let claude_dir = tmp.path().join("claude");
        let dir = claude_dir.join("projects/-tmp-demo");
        std::fs::create_dir_all(&dir).unwrap();
        let t = dir.join("abc.jsonl");
        std::fs::write(&t, "{\"a\":1}\n".repeat(100)).unwrap();
        db.conn
            .execute(
                "INSERT INTO session(id,project_dir,transcript_path,transcript_state) VALUES('abc','-tmp-demo',?1,'present')",
                [t.to_string_lossy()],
            )
            .unwrap();
        let agents = Agents::from_vec(vec![Arc::new(Claude::new(claude_dir))]);
        (tmp, paths, db, t, agents)
    }

    #[test]
    fn archive_then_restore_round_trip() {
        let (_tmp, paths, db, t, agents) = setup();
        let original = std::fs::read(&t).unwrap();
        assert!(matches!(
            archive_session(&db, &paths, &agents, "abc").unwrap(),
            Archived::Copied(_)
        ));
        assert_eq!(
            archive_session(&db, &paths, &agents, "abc").unwrap(),
            Archived::Unchanged
        );
        let s = db.session_by_id("abc").unwrap().unwrap();
        let copy = PathBuf::from(s.archive_path.unwrap());
        assert!(copy.ends_with("-tmp-demo/abc.jsonl.zst"));
        // the same bytes earlier builds wrote, so existing copies and new ones are one kind
        assert_eq!(
            std::fs::read(&copy).unwrap(),
            zstd::encode_all(&original[..], LEVEL).unwrap()
        );
        // refuses while the original exists
        assert!(restore(&db, &paths, &agents, "abc").is_err());
        std::fs::remove_file(&t).unwrap();
        assert_eq!(restore(&db, &paths, &agents, "abc").unwrap(), t);
        assert_eq!(std::fs::read(&t).unwrap(), original);
    }

    #[test]
    fn a_deleted_transcript_comes_back_only_when_a_copy_exists() {
        let (_tmp, paths, db, t, agents) = setup();
        let session = |db: &Db| db.session_by_id("abc").unwrap().unwrap();
        // no copy yet: nothing to restore, and no error
        std::fs::remove_file(&t).unwrap();
        restore_if_missing(&db, &paths, &agents, &session(&db)).unwrap();
        assert!(!t.exists());
        std::fs::write(&t, "{\"a\":1}\n").unwrap();
        archive_session(&db, &paths, &agents, "abc").unwrap();
        // the transcript is there: left alone
        restore_if_missing(&db, &paths, &agents, &session(&db)).unwrap();
        std::fs::remove_file(&t).unwrap();
        restore_if_missing(&db, &paths, &agents, &session(&db)).unwrap();
        assert_eq!(std::fs::read_to_string(&t).unwrap(), "{\"a\":1}\n");
    }

    #[test]
    fn changed_transcript_is_copied_again() {
        let (_tmp, paths, db, t, agents) = setup();
        archive_session(&db, &paths, &agents, "abc").unwrap();
        let mut more = std::fs::read(&t).unwrap();
        more.extend_from_slice(b"{\"b\":2}\n");
        std::fs::write(&t, more).unwrap();
        assert!(matches!(
            archive_session(&db, &paths, &agents, "abc").unwrap(),
            Archived::Copied(_)
        ));
        assert!(matches!(
            archive_all(&db, &paths, &agents).unwrap()[0].1,
            Archived::Unchanged
        ));
    }

    #[test]
    fn a_deleted_transcript_is_read_from_a_temporary_copy_that_goes_away() {
        let (_tmp, paths, db, t, agents) = setup();
        let original = std::fs::read(&t).unwrap();
        archive_session(&db, &paths, &agents, "abc").unwrap();
        let s = db.session_by_id("abc").unwrap().unwrap();
        let agent = agents.of_session(&s).unwrap();
        // while the transcript is there it is read in place
        let live = readable(&s, &paths, agent.as_ref()).unwrap();
        assert_eq!(live.path, t);
        drop(live);
        assert!(t.exists());
        std::fs::remove_file(&t).unwrap();
        let copy = readable(&s, &paths, agent.as_ref()).unwrap();
        assert_eq!(std::fs::read(&copy.path).unwrap(), original);
        assert_eq!(copy.path.file_name(), t.file_name());
        assert!(copy.path.starts_with(paths.home.join("tmp")));
        let (file, folder) = (copy.path.clone(), copy.path.parent().unwrap().to_path_buf());
        drop(copy);
        assert!(!file.exists() && !folder.exists());
        // no transcript and no copy: nothing to read
        let none = Session {
            id: "zzz".into(),
            ..Default::default()
        };
        assert!(readable(&none, &paths, agent.as_ref()).is_err());
    }
}
