//! `eye forget`: take sessions out of eye, and with `transcript` out of their agent too. What is
//! removed is decided here; how an agent's own copy is removed is the adapter's call
//! (`Agent::delete_transcript`). A snapshot of the database is taken first.

use crate::agents::{Agent, Agents};
use crate::backup::{self, Reason};
use crate::config::Config;
use crate::db::Db;
use crate::model::Session;
use crate::paths::Paths;
use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// What happened to the agent's own copy of one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnCopy {
    /// Nobody asked for it to go.
    Kept,
    /// Removed; these are the places.
    Removed(Vec<String>),
    /// The agent's adapter does not delete sessions, or the agent is switched off.
    Unsupported(String),
    /// Removing it failed; the session is still in eye, so the command can be run again.
    Failed(String),
}

/// What forgetting one session did.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub id: String,
    pub title: String,
    pub own: OwnCopy,
    /// The session is out of eye's index (false when removing the agent's copy failed).
    pub forgotten: bool,
    /// eye's own archive copy was found and removed (false when there was none).
    pub archive_removed: bool,
}

#[derive(Debug)]
pub struct Done {
    pub snapshot: PathBuf,
    pub outcomes: Vec<Outcome>,
}

/// The sessions with these ids, then none twice.
pub fn by_ids(db: &Db, ids: &[String]) -> Result<Vec<Session>> {
    let mut out: Vec<Session> = vec![];
    for i in ids {
        let id = db.resolve_id(i)?;
        if out.iter().all(|s| s.id != id) {
            out.push(db.session_by_id(&id)?.context("session vanished")?);
        }
    }
    Ok(out)
}

/// Every session whose folder is `folder` or inside it.
pub fn by_folder(db: &Db, folder: &Path) -> Result<Vec<Session>> {
    let mut want = vec![folder.to_path_buf()];
    if let Ok(real) = folder.canonicalize() {
        want.push(real);
    }
    let mut out: Vec<Session> = db
        .all_sessions()?
        .into_iter()
        .filter(|s| {
            s.cwd
                .as_deref()
                .is_some_and(|c| want.iter().any(|w| Path::new(c).starts_with(w)))
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.last_activity_at));
    Ok(out)
}

/// Is the session open in its agent right now? Those are never forgotten.
pub fn is_open(s: &Session) -> bool {
    s.live_status.is_some()
}

/// The place of eye's archive copy of a session, when there is one.
pub fn archive_copy(s: &Session, paths: &Paths, agents: &Agents) -> Option<PathBuf> {
    archive_copy_in(s, paths, agents, None)
}

/// As `archive_copy`; `known` (every registered agent, enabled or not) gives the archive layout of
/// a session whose agent is switched off, when the stored path lies outside this archive (a
/// session that came in from a pack).
fn archive_copy_in(
    s: &Session,
    paths: &Paths,
    agents: &Agents,
    known: Option<&Agents>,
) -> Option<PathBuf> {
    let archive_root = paths.home.join("archive");
    let mut spots: Vec<PathBuf> = s.archive_path.iter().map(PathBuf::from).collect();
    let layout = agents
        .of_session(s)
        .or_else(|| known.and_then(|k| k.of_session(s)));
    if let (Some(a), Some(pd)) = (layout, s.project_dir.as_deref()) {
        spots.push(crate::archive::archive_path(paths, a.as_ref(), pd, &s.id));
    }
    spots.into_iter().find(|p| {
        crate::agents::is_inside(&archive_root, p) && std::fs::symlink_metadata(p).is_ok()
    })
}

/// Whether the agent of `s` can delete its own copy: its adapter says so only by trying, so this
/// asks for the agents that are enabled and leaves the answer to `forget`.
fn agent_of<'a>(agents: &'a Agents, s: &Session) -> Option<&'a dyn Agent> {
    agents.of_session(s).map(|a| a.as_ref())
}

/// Remove the sessions from eye (and their agent's copy when `transcript`). Refuses, before it
/// changes anything, when one of them is open now. Takes a manual snapshot first.
pub fn forget(
    db: &Db,
    paths: &Paths,
    cfg: &Config,
    agents: &Agents,
    sessions: &[Session],
    transcript: bool,
) -> Result<Done> {
    let open: Vec<&str> = sessions
        .iter()
        .filter(|s| is_open(s))
        .map(|s| s.id.as_str())
        .collect();
    if !open.is_empty() {
        bail!(
            "open now, so not forgotten: {} (close them first)",
            open.join(", ")
        );
    }
    let snap = backup::take(&db.conn, paths, &cfg.backup, &Reason::Manual)
        .context("making a snapshot before forgetting anything")?;
    let known = crate::agents::known(cfg);
    let mut outcomes = vec![];
    for s in sessions {
        let title = s.title.clone().unwrap_or_default();
        let own = if transcript {
            own_copy(agent_of(agents, s), s)
        } else {
            OwnCopy::Kept
        };
        if let OwnCopy::Failed(_) = own {
            outcomes.push(Outcome {
                id: s.id.clone(),
                title,
                own,
                forgotten: false,
                archive_removed: false,
            });
            continue;
        }
        // the archive copy goes only once the rows are really gone
        let copy = archive_copy_in(s, paths, agents, Some(&known));
        remove_rows(&db.conn, s, transcript)?;
        let mut archive_removed = false;
        if let Some(copy) = copy {
            let gone = if std::fs::symlink_metadata(&copy).is_ok_and(|m| m.is_dir()) {
                std::fs::remove_dir_all(&copy)
            } else {
                std::fs::remove_file(&copy)
            };
            match gone {
                Ok(()) => archive_removed = true,
                Err(e) => eprintln!("warn: could not remove {}: {e}", copy.display()),
            }
        }
        outcomes.push(Outcome {
            id: s.id.clone(),
            title,
            own,
            forgotten: true,
            archive_removed,
        });
    }
    Ok(Done {
        snapshot: snap.path,
        outcomes,
    })
}

fn own_copy(agent: Option<&dyn Agent>, s: &Session) -> OwnCopy {
    let Some(agent) = agent else {
        return OwnCopy::Unsupported(format!("{} is not enabled", s.agent));
    };
    let Some(path) = s.transcript_path.as_deref() else {
        return OwnCopy::Removed(vec![]);
    };
    match agent.delete_transcript(Path::new(path), &s.id) {
        Ok(Some(gone)) => OwnCopy::Removed(gone),
        Ok(None) => OwnCopy::Unsupported(format!("{} does not support it yet", agent.name())),
        Err(e) => OwnCopy::Failed(format!("{e:#}")),
    }
}

/// The tables that have a `session_id` column (the session text index included), found from the
/// schema as `eye import` does.
fn session_tables(conn: &Connection) -> Result<Vec<String>> {
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = vec![];
    for t in names {
        if t == "session" || t.starts_with("sqlite_") {
            continue;
        }
        let has = conn
            .prepare("SELECT 1 FROM pragma_table_info(?1) WHERE name='session_id'")?
            .exists([&t])?;
        if has {
            out.push(t);
        }
    }
    Ok(out)
}

/// Delete everything eye holds about one session, in one transaction. With `tombstone` the agent's
/// history file is told not to bring the session back as a history-only one.
fn remove_rows(conn: &Connection, s: &Session, tombstone: bool) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for t in session_tables(&tx)? {
        match t.as_str() {
            // ideas are the person's own list: they stay, without the link to the session
            "idea" => {
                tx.execute(
                    "UPDATE idea SET session_id=NULL WHERE session_id=?1",
                    [&s.id],
                )?;
                tx.execute(
                    "UPDATE idea SET from_session_id=NULL WHERE from_session_id=?1",
                    [&s.id],
                )?;
            }
            _ => {
                tx.execute(&format!("DELETE FROM \"{t}\" WHERE session_id=?1"), [&s.id])?;
            }
        }
    }
    tx.execute(
        "UPDATE night_run SET run_session_id=NULL WHERE run_session_id=?1",
        [&s.id],
    )?;
    tx.execute(
        "UPDATE session SET parent_session_id=NULL WHERE parent_session_id=?1",
        [&s.id],
    )?;
    tx.execute("DELETE FROM session WHERE id=?1", [&s.id])?;
    if let Some(tp) = s.transcript_path.as_deref() {
        // the transcript's cursor, and those of the files below its folder (sub-agents)
        let base = tp.strip_suffix(".jsonl").unwrap_or(tp);
        tx.execute(
            "DELETE FROM cursor WHERE path=?1 OR substr(path,1,length(?2)+1)=?2||'/'",
            [tp, base],
        )?;
        tx.execute(
            "DELETE FROM meta WHERE key IN (?1,?2) OR substr(key,1,length(?3)+1)=?3||'/' \
             OR substr(key,1,length(?4)+1)=?4||'/'",
            [
                &format!("pending:{tp}"),
                &format!("subtok:{tp}"),
                &format!("pending:{base}"),
                &format!("subtok:{base}"),
            ],
        )?;
    }
    tx.execute(
        "DELETE FROM meta WHERE key IN (?1,?2,?3)",
        [
            format!("archived_sig:{}", s.id),
            format!("quiet_tried:{}", s.id),
            format!("folders:{}", s.id),
        ],
    )?;
    if tombstone {
        crate::db::meta_set(&tx, &format!("forgotten:{}", s.id), "1")?;
    }
    tx.commit()?;
    Ok(())
}

/// Did `eye forget --transcript` remove this session? Then a history file that still lists its
/// prompts must not bring it back.
pub fn is_forgotten(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM meta WHERE key=?1",
        [format!("forgotten:{id}")],
        |_| Ok(()),
    )
    .is_ok()
}
