//! `eye import`: check a pack, say what it would do, and do it. On a machine with no sessions the
//! pack's database becomes the database; otherwise the sessions and ideas the pack has and this
//! machine lacks are merged in. Paths are rewritten for this machine first.

use super::remap::Remap;
use super::{Manifest, Scratch};
use crate::agents::LEGACY_ID;
use crate::backup::{self, Applied, Reason};
use crate::config::Config;
use crate::db::{Db, SCHEMA_VERSION};
use crate::install::Opts;
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use std::collections::BTreeSet;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// Tables that are never merged session by session, though they may have a `session_id` column.
/// A night run belongs to the machine that ran it: its folder, worktree and process mean nothing
/// on another one, and its idea is numbered by that machine's ideas.
const NOT_MERGED: [&str; 5] = ["event", "cursor", "meta", "idea", "night_run"];

/// A checked pack and what importing it would change. Holds the unpacked files until dropped.
pub struct Plan {
    pub manifest: Manifest,
    /// This machine has no sessions: the pack's database becomes the database.
    pub fresh: bool,
    pub remap: Remap,
    pub new_sessions: Vec<String>,
    pub updated_sessions: Vec<String>,
    pub unchanged_sessions: usize,
    pub new_ideas: usize,
    /// Files (archive and live) that would be copied, and files already here and as new.
    pub files_copy: usize,
    pub files_have: usize,
    /// Sessions that would come in whose folder does not exist on this machine.
    pub folders_missing: usize,
    _scratch: Scratch,
    staged_db: PathBuf,
    /// (name in the pack, where it goes, mtime) of each file to copy.
    copies: Vec<(PathBuf, PathBuf, i64)>,
}

/// The unpacked pack, as the CLI names it.
pub type Prepared = Plan;

impl Plan {
    /// The plan as lines for people.
    pub fn lines(&self, paths: &Paths) -> Vec<String> {
        let m = &self.manifest;
        let mut v = vec![format!(
            "pack from {} ({}), made {}, eye {} schema {}",
            m.host,
            m.user,
            crate::util::fmt_ts(m.created_at),
            m.eye_version,
            m.schema
        )];
        if self.fresh {
            v.push(format!(
                "- this machine has no sessions: the pack's database becomes the database ({} sessions, {} ideas)",
                m.counts.sessions, m.counts.ideas
            ));
        } else {
            v.push(format!(
                "- merge into the {} sessions here: {} new, {} updated (the pack's copy is newer), {} unchanged",
                self.unchanged_sessions + self.updated_sessions.len(),
                self.new_sessions.len(),
                self.updated_sessions.len(),
                self.unchanged_sessions
            ));
            v.push(format!("- {} new idea(s)", self.new_ideas));
        }
        v.push(format!(
            "- {} transcript copies to put in {} ({} already here), {} of them from the agent's own folder on the other machine",
            self.files_copy,
            crate::util::show_path(&paths.home.join("archive")),
            self.files_have,
            m.counts.live_files
        ));
        if self.remap.is_empty() {
            v.push("- paths: unchanged (same home folder)".into());
        } else {
            for (from, to) in self.remap.pairs() {
                v.push(format!(
                    "- paths: {from} -> {to} (also in Claude's encoded form)"
                ));
            }
        }
        if self.folders_missing > 0 {
            v.push(format!(
                "- {} session(s) are in folders that do not exist here; they open from the archive but cannot be resumed until the folder is there",
                self.folders_missing
            ));
        }
        v.push("- the current database is kept first as a pre-import snapshot".into());
        v.push("- the daemon is stopped while the database changes, and started again".into());
        v
    }
}

fn has_col(conn: &Connection, table: &str, col: &str) -> bool {
    conn.prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name=?2")
        .and_then(|mut s| s.exists(params![table, col]))
        .unwrap_or(false)
}

/// Where a file of the pack goes inside `archive/`, project folder names rewritten. `live/<agent>/…`
/// goes where that agent's archive copies are (`@<agent>/…`, none for the first agent).
fn archive_rel(name: &str, remap: &Remap) -> Option<PathBuf> {
    let rel = match (name.strip_prefix("archive/"), name.strip_prefix("live/")) {
        (Some(r), _) => r.to_string(),
        (None, Some(r)) => {
            let (agent, rest) = r.split_once('/')?;
            if agent == LEGACY_ID {
                rest.to_string()
            } else {
                format!("@{agent}/{rest}")
            }
        }
        _ => return None,
    };
    let mut parts: Vec<String> = rel.split('/').map(str::to_string).collect();
    let at = usize::from(parts.first().is_some_and(|p| p.starts_with('@')));
    if parts.len() > at + 1 {
        parts[at] = remap.project_dir(&parts[at]);
    }
    Some(parts.iter().collect())
}

/// `path` (already moved to this home) with the project folder it sits in renamed too.
fn in_project_dir(path: &str, old_pd: Option<&str>, remap: &Remap) -> String {
    let p = Path::new(path);
    let (Some(old), Some(parent)) = (old_pd, p.parent()) else {
        return path.to_string();
    };
    let new = remap.project_dir(old);
    // the home swap may already have changed it (`.../-Users-max-x` is not a path prefix)
    if parent.file_name().is_some_and(|n| n == old) && new != old {
        return parent
            .with_file_name(&new)
            .join(p.file_name().unwrap_or_default())
            .to_string_lossy()
            .into_owned();
    }
    path.to_string()
}

/// Where a session's archive copy (an absolute path on the other machine) is on this one. The
/// pack's eye home is not always the folder the path says (`EYE_HOME`, or the `~/.claude-eye` of
/// before the rename), so the part after `…/archive/` is taken and put under this machine's own.
fn archive_path_here(path: &str, m: &Manifest, root: &Path, remap: &Remap) -> Option<String> {
    let roots = [
        m.eye_home.clone(),
        format!("{}/.agent-eye", m.home),
        format!("{}/.claude-eye", m.home),
    ];
    roots.iter().find_map(|r| {
        let rel = path.strip_prefix(&format!("{}/archive/", r.trim_end_matches('/')))?;
        let rel = archive_rel(&format!("archive/{rel}"), remap)?;
        Some(
            root.join("archive")
                .join(rel)
                .to_string_lossy()
                .into_owned(),
        )
    })
}

/// Rewrite the paths of the staged database for this machine, and say where each session's copy
/// of the transcript is and whether the agent still has the transcript.
fn prepare_db(db: &Path, remap: &Remap, m: &Manifest, root: &Path) -> Result<()> {
    let conn = Connection::open(db).context("opening the pack's database")?;
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    if !remap.is_empty() {
        let mut sel = conn.prepare(
            "SELECT id, cwd, repo_root, transcript_path, archive_path, project_dir FROM session",
        )?;
        let rows: Vec<(String, [Option<String>; 5])> = sel
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(sel);
        for (id, vals) in rows {
            let old_pd = vals[4].clone();
            let new: Vec<Option<String>> = vals
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    v.as_deref().map(|v| match i {
                        4 => remap.project_dir(v),
                        // the agent's folder for the project is named after the path: so is the
                        // folder the transcript and its copy sit in
                        2 => in_project_dir(&remap.path(v), old_pd.as_deref(), remap),
                        3 => archive_path_here(v, m, root, remap).unwrap_or_else(|| {
                            in_project_dir(&remap.path(v), old_pd.as_deref(), remap)
                        }),
                        _ => remap.path(v),
                    })
                })
                .collect();
            if new.iter().zip(vals.iter()).any(|(a, b)| a != b) {
                conn.execute(
                    "UPDATE session SET cwd=?2, repo_root=?3, transcript_path=?4, archive_path=?5, project_dir=?6 WHERE id=?1",
                    params![id, new[0], new[1], new[2], new[3], new[4]],
                )?;
            }
        }
        if has_col(&conn, "idea", "folder") {
            let ideas: Vec<(i64, String)> = conn
                .prepare("SELECT id, folder FROM idea WHERE folder IS NOT NULL")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, f) in ideas {
                let n = remap.path(&f);
                if n != f {
                    conn.execute("UPDATE idea SET folder=?2 WHERE id=?1", params![id, n])?;
                }
            }
        }
    }
    // the transcripts that came as live copies now have an archive copy here
    for l in &m.live {
        if let Some(rel) = archive_rel(&l.file, remap) {
            conn.execute(
                "UPDATE session SET archive_path=?2 WHERE id=?1",
                params![l.id, root.join("archive").join(rel).to_string_lossy()],
            )?;
        }
    }
    // a transcript that is not on this machine is the archive's, or gone
    let rows: Vec<(String, String, Option<String>)> = conn
        .prepare("SELECT id, transcript_path, archive_path FROM session WHERE transcript_path IS NOT NULL AND transcript_state='present'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, tp, ap) in rows {
        if !Path::new(&tp).exists() {
            let state = if ap.is_some() { "archived" } else { "deleted" };
            conn.execute(
                "UPDATE session SET transcript_state=?2 WHERE id=?1",
                params![id, state],
            )?;
        }
    }
    Ok(())
}

/// Unpack and check the pack, work out what importing it changes on this machine. Nothing outside
/// the eye home's temp folder is written.
pub fn plan(paths: &Paths, pack: &Path) -> Result<Plan> {
    paths.ensure_home()?;
    let scratch = Scratch::new(&paths.home, "import")?;
    let dir = scratch.dir.join("pack");
    let unpacked = super::read(pack, &dir)?;
    let manifest = unpacked.manifest;
    let staged_db = dir.join("eye.db");
    {
        let c = Connection::open_with_flags(&staged_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("opening the pack's database")?;
        if let Some(found) = backup::quick_check(&c) {
            bail!("the pack's database is damaged ({found})");
        }
        let schema = backup::schema_of(&c)?;
        if schema > SCHEMA_VERSION {
            bail!(
                "the pack's database is schema {schema}, this eye knows {SCHEMA_VERSION}; update eye first"
            );
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let eye = paths.home.to_string_lossy().into_owned();
    let remap = Remap::new(&manifest.home, &home, &manifest.eye_home, &eye);
    prepare_db(&staged_db, &remap, &manifest, &paths.home)?;

    let local = paths.db_path();
    let local_sessions: i64 = if local.exists() {
        Connection::open_with_flags(&local, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
            .and_then(|c| {
                c.query_row("SELECT COUNT(*) FROM session", [], |r| r.get(0))
                    .ok()
            })
            .unwrap_or(0)
    } else {
        0
    };
    let fresh = local_sessions == 0;

    let pk = Connection::open(&staged_db)?;
    let (new_sessions, updated_sessions, unchanged, new_ideas): (
        Vec<String>,
        Vec<String>,
        usize,
        usize,
    );
    let cwds = |ids: &[String]| -> usize {
        ids.iter()
            .filter(|id| {
                pk.query_row("SELECT cwd FROM session WHERE id=?1", [id], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .ok()
                .flatten()
                .is_some_and(|c| !Path::new(&c).is_dir())
            })
            .count()
    };
    if fresh {
        new_sessions = pk
            .prepare("SELECT id FROM session")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        updated_sessions = vec![];
        unchanged = 0;
        new_ideas = manifest.counts.ideas as usize;
    } else {
        pk.execute(
            "ATTACH DATABASE ?1 AS loc",
            [format!("file:{}?mode=ro", local.display())],
        )?;
        let ids = |sql: &str| -> Result<Vec<String>> {
            Ok(pk
                .prepare(sql)?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        };
        new_sessions =
            ids("SELECT id FROM main.session WHERE id NOT IN (SELECT id FROM loc.session)")?;
        updated_sessions = ids(
            "SELECT p.id FROM main.session p JOIN loc.session l ON l.id=p.id
             WHERE COALESCE(p.last_activity_at,0) > COALESCE(l.last_activity_at,0)",
        )?;
        let total: i64 = pk.query_row(
            "SELECT COUNT(*) FROM main.session p JOIN loc.session l ON l.id=p.id",
            [],
            |r| r.get(0),
        )?;
        unchanged = total as usize - updated_sessions.len();
        new_ideas = pk.query_row(
            "SELECT COUNT(*) FROM main.idea p WHERE NOT EXISTS (SELECT 1 FROM loc.idea l
               WHERE l.title IS p.title AND l.folder IS p.folder AND l.created_at IS p.created_at)",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;
    }
    let folders_missing = cwds(&new_sessions) + cwds(&updated_sessions);
    drop(pk);

    let root = paths.home.join("archive");
    let (mut copies, mut have) = (vec![], 0usize);
    for (name, sum) in &manifest.files {
        let Some(rel) = archive_rel(name, &remap) else {
            continue;
        };
        let dest = root.join(rel);
        let newer = match std::fs::metadata(&dest) {
            // an older copy that holds the same bytes needs no copying
            Ok(md) if super::mtime_secs(&md) < sum.mtime => !same_file(&dest, md.len(), sum),
            Ok(_) => false,
            Err(_) => true,
        };
        if newer {
            copies.push((dir.join(name), dest, sum.mtime));
        } else {
            have += 1;
        }
    }
    Ok(Plan {
        manifest,
        fresh,
        remap,
        new_sessions,
        updated_sessions,
        unchanged_sessions: unchanged,
        new_ideas,
        files_copy: copies.len(),
        files_have: have,
        folders_missing,
        _scratch: scratch,
        staged_db,
        copies,
    })
}

/// Does the file at `path` (`len` bytes) have the sha256 and size the pack lists?
fn same_file(path: &Path, len: u64, sum: &super::Sum) -> bool {
    len == sum.bytes
        && std::fs::File::open(path)
            .and_then(super::hash_reader)
            .is_ok_and(|(h, _)| h == sum.sha256)
}

/// Copy a file into place (0600, folders 0700), keeping its modification time.
fn put(src: &Path, dest: &Path, mtime: i64) -> Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dest.parent().context("no parent")?)?;
    let tmp = dest.with_file_name(format!(
        "{}.tmp",
        dest.file_name().unwrap_or_default().to_string_lossy()
    ));
    {
        let mut from = std::fs::File::open(src)?;
        let mut to = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        std::io::copy(&mut from, &mut to)?;
        to.set_modified(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime.max(0) as u64),
        )?;
        to.sync_all()?;
    }
    std::fs::rename(&tmp, dest).with_context(|| format!("writing {}", dest.display()))
}

/// What an import did.
#[derive(Debug)]
pub struct Done {
    pub applied: Applied,
    pub files_copied: usize,
    pub sessions_added: usize,
    pub sessions_updated: usize,
    pub ideas_added: usize,
}

/// Do the plan: copy the files, then change the database with the daemon stopped.
pub fn apply(o: &Opts, cfg: &Config, plan: Plan) -> Result<Done> {
    let paths = Paths {
        home: o.eye_home.clone(),
    };
    // The snapshot comes before the first file is copied, so a copy that fails leaves the old
    // database kept. (A fresh machine has an empty database: `replace_with` snapshots it.)
    let pre = if plan.fresh {
        None
    } else {
        backup::pre_restore_copy(&paths, cfg, &paths.db_path(), &Reason::PreImport)?
    };
    let mut files_copied = 0;
    for (src, dest, mtime) in &plan.copies {
        put(src, dest, *mtime)?;
        files_copied += 1;
    }
    if plan.fresh {
        // an empty database is made first, so the snapshot before the import exists
        drop(Db::open(&paths.db_path())?);
        let applied = backup::replace_with(o, cfg, &plan.staged_db, &Reason::PreImport)?;
        return Ok(Done {
            applied,
            files_copied,
            sessions_added: plan.new_sessions.len(),
            sessions_updated: 0,
            ideas_added: plan.manifest.counts.ideas as usize,
        });
    }
    let take: Vec<&String> = plan
        .new_sessions
        .iter()
        .chain(plan.updated_sessions.iter())
        .collect();
    let (ideas_added, mut applied) = backup::while_stopped(o, || {
        let db = Db::open(&paths.db_path())?;
        merge(&db.conn, &plan.staged_db, &take)
    })?;
    applied.pre_restore = pre;
    Ok(Done {
        applied,
        files_copied,
        sessions_added: plan.new_sessions.len(),
        sessions_updated: plan.updated_sessions.len(),
        ideas_added,
    })
}

fn common_cols(conn: &Connection, table: &str, skip_id: bool) -> Result<Vec<String>> {
    let cols = |schema: &str| -> Result<BTreeSet<String>> {
        Ok(conn
            .prepare("SELECT name FROM pragma_table_info(?1, ?2)")?
            .query_map(params![table, schema], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?)
    };
    let (main, pk) = (cols("main")?, cols("pk")?);
    Ok(pk
        .into_iter()
        .filter(|c| main.contains(c) && !(skip_id && c == "id"))
        .collect())
}

/// Tables of both databases that hold rows of sessions (`session_id`), except the ones that are
/// not merged and the session's own text index, which is rebuilt row by row.
fn child_tables(conn: &Connection) -> Result<Vec<String>> {
    let names: Vec<String> = conn
        .prepare("SELECT name FROM pk.sqlite_master WHERE type='table'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = vec![];
    for t in names {
        if t == "session" || t.starts_with("session_fts") || t.starts_with("sqlite_") {
            continue;
        }
        if NOT_MERGED.contains(&t.as_str()) {
            continue;
        }
        let both = |schema: &str| {
            conn.prepare("SELECT 1 FROM pragma_table_info(?1, ?2) WHERE name='session_id'")
                .and_then(|mut s| s.exists(params![t, schema]))
                .unwrap_or(false)
        };
        let exists_main = conn
            .prepare("SELECT 1 FROM main.sqlite_master WHERE type='table' AND name=?1")?
            .exists([&t])?;
        if exists_main && both("main") && both("pk") {
            out.push(t);
        }
    }
    Ok(out)
}

/// Merge the pack's database into the local one: the sessions in `take` (new or newer in the
/// pack) with their rows, and the ideas that are not here yet. Returns the ideas added.
fn merge(local: &Connection, staged: &Path, take: &[&String]) -> Result<usize> {
    local.execute("ATTACH DATABASE ?1 AS pk", [staged.to_string_lossy()])?;
    let r = (|| -> Result<usize> {
        let tx = local.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS take(id TEXT PRIMARY KEY); DELETE FROM temp.take;",
        )?;
        {
            let mut ins = tx.prepare("INSERT OR IGNORE INTO temp.take(id) VALUES(?1)")?;
            for id in take {
                ins.execute([id])?;
            }
        }
        let children = child_tables(local)?;
        tx.execute(
            "DELETE FROM main.session_fts WHERE rowid IN (SELECT rowid FROM main.session WHERE id IN (SELECT id FROM temp.take))",
            [],
        )?;
        for t in &children {
            tx.execute(
                &format!("DELETE FROM main.\"{t}\" WHERE session_id IN (SELECT id FROM temp.take)"),
                [],
            )?;
        }
        tx.execute(
            "DELETE FROM main.session WHERE id IN (SELECT id FROM temp.take)",
            [],
        )?;
        let cols = common_cols(local, "session", false)?.join(",");
        tx.execute(
            &format!("INSERT INTO main.session({cols}) SELECT {cols} FROM pk.session WHERE id IN (SELECT id FROM temp.take)"),
            [],
        )?;
        for t in &children {
            let cols = common_cols(local, t, true)?
                .into_iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(",");
            tx.execute(
                &format!("INSERT OR REPLACE INTO main.\"{t}\"({cols}) SELECT {cols} FROM pk.\"{t}\" WHERE session_id IN (SELECT id FROM temp.take)"),
                [],
            )?;
        }
        // the text index keeps the session's rowid, so a lookup never scans
        tx.execute(
            "INSERT INTO main.session_fts(rowid, session_id, title, prompts, replies, extra)
             SELECT (SELECT rowid FROM main.session s WHERE s.id=f.session_id), f.session_id, f.title, f.prompts, f.replies, f.extra
             FROM pk.session_fts f WHERE f.session_id IN (SELECT id FROM temp.take)
               AND EXISTS (SELECT 1 FROM main.session s WHERE s.id=f.session_id)",
            [],
        )?;
        let icols = common_cols(local, "idea", true)?
            .into_iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(",");
        let ideas = tx.execute(
            &format!(
                "INSERT INTO main.idea({icols}) SELECT {icols} FROM pk.idea p WHERE NOT EXISTS
                 (SELECT 1 FROM main.idea l WHERE l.title IS p.title AND l.folder IS p.folder AND l.created_at IS p.created_at)"
            ),
            [],
        )?;
        // an idea's number differs between the two databases: point sessions at the same idea here
        if common_cols(local, "session", false)?
            .iter()
            .any(|c| c == "idea_id")
        {
            tx.execute(
                "UPDATE main.session SET idea_id = (
                   SELECT l.id FROM pk.session ps JOIN pk.idea p ON p.id=ps.idea_id
                   JOIN main.idea l ON l.title IS p.title AND l.folder IS p.folder AND l.created_at IS p.created_at
                   WHERE ps.id = main.session.id)
                 WHERE id IN (SELECT id FROM temp.take)",
                [],
            )?;
        }
        tx.execute_batch("DROP TABLE temp.take;")?;
        tx.commit()?;
        Ok(ideas)
    })();
    let _ = local.execute("DETACH DATABASE pk", []);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::{Counts, Live};
    use std::collections::BTreeMap;

    fn manifest() -> Manifest {
        Manifest {
            format: 1,
            eye_version: "0".into(),
            schema: 1,
            created_at: 0,
            host: "h".into(),
            home: "/Users/max".into(),
            eye_home: "/tmp/elsewhere/eye".into(),
            user: "max".into(),
            counts: Counts::default(),
            live: Vec::<Live>::new(),
            files: BTreeMap::new(),
        }
    }

    #[test]
    fn archive_files_land_in_this_archive_with_the_project_folder_renamed() {
        let r = Remap::new(
            "/Users/max",
            "/home/max",
            "/tmp/elsewhere/eye",
            "/home/max/.agent-eye",
        );
        let p = |n: &str| archive_rel(n, &r).unwrap().to_string_lossy().into_owned();
        assert_eq!(
            p("archive/-Users-max-code/a.jsonl.zst"),
            "-home-max-code/a.jsonl.zst"
        );
        assert_eq!(p("archive/@codex/2026/11/a.zst"), "@codex/2026/11/a.zst");
        assert_eq!(
            p("live/claude/-Users-max-x/a.jsonl.zst"),
            "-home-max-x/a.jsonl.zst"
        );
        assert_eq!(p("live/codex/2026-11/a.zst"), "@codex/2026-11/a.zst");
        assert!(archive_rel("eye.db", &r).is_none());
        // a stored archive path is found whichever eye home it names, the old `.claude-eye` too
        let root = Path::new("/home/max/.agent-eye");
        for old in [
            "/tmp/elsewhere/eye/archive/-Users-max-code/a.jsonl.zst",
            "/Users/max/.agent-eye/archive/-Users-max-code/a.jsonl.zst",
            "/Users/max/.claude-eye/archive/-Users-max-code/a.jsonl.zst",
        ] {
            assert_eq!(
                archive_path_here(old, &manifest(), root, &r).as_deref(),
                Some("/home/max/.agent-eye/archive/-home-max-code/a.jsonl.zst"),
                "{old}"
            );
        }
        assert_eq!(
            archive_path_here("/elsewhere/a.zst", &manifest(), root, &r),
            None
        );
    }

    #[test]
    fn a_transcript_s_folder_is_renamed_with_the_project() {
        let r = Remap::new("/Users/max", "/home/max", "", "");
        assert_eq!(
            in_project_dir(
                "/home/max/.claude/projects/-Users-max-code/a.jsonl",
                Some("-Users-max-code"),
                &r
            ),
            "/home/max/.claude/projects/-home-max-code/a.jsonl"
        );
        assert_eq!(in_project_dir("/x/y/a.jsonl", None, &r), "/x/y/a.jsonl");
    }
}
