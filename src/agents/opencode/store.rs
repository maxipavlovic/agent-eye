//! Where OpenCode keeps sessions and how they are listed and read. Three layouts exist in the
//! wild, all under the data folder (`$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode`):
//!
//! - `opencode.db` (or `opencode-<channel>.db`, or `$OPENCODE_DB`): tables `session_v2`,
//!   `session_message`, `session_part` (v2.x), and the older `session`, `message`, `part` tables
//!   (v1.2 and later, kept next to the v2 ones after the migration);
//! - `storage/session/<project>/<id>.json`, `storage/message/<session>/<id>.json` and
//!   `storage/part/<message>/<id>.json`: the files of v1.0 and v1.1, before the database.
//!
//! A session of a database is named by the path `<db>#<session id>`, which is no file; a session of
//! the file layout by its `storage/session/...` file.

use super::events::{from_v1, from_v2, Ev, Loaded};
use super::turns;
use crate::agents::Transcript;
use crate::model::Stamp;
use crate::util::valid_session_id;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(super) fn open(path: &Path) -> Option<Connection> {
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    c.busy_timeout(std::time::Duration::from_millis(300)).ok()?;
    Some(c)
}

pub(super) fn has_column(c: &Connection, table: &str, column: &str) -> bool {
    c.query_row(
        "SELECT 1 FROM pragma_table_info(?1) WHERE name=?2",
        [table, column],
        |_| Ok(()),
    )
    .is_ok()
}

fn has_table(c: &Connection, name: &str) -> bool {
    c.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |_| Ok(()),
    )
    .is_ok()
}

/// The path that names session `id` of the database `db`.
pub fn db_path(db: &Path, id: &str) -> PathBuf {
    PathBuf::from(format!("{}#{id}", db.display()))
}

/// The database and the session id of a path made by [`db_path`].
pub fn split_db_path(p: &Path) -> Option<(PathBuf, String)> {
    let s = p.to_str()?;
    let (db, id) = s.rsplit_once('#')?;
    (db.ends_with(".db") && !id.is_empty()).then(|| (PathBuf::from(db), id.to_string()))
}

/// The databases of a data folder.
pub(super) fn databases(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("opencode") && n.ends_with(".db"))
        })
        .collect();
    if let Some(v) = std::env::var_os("OPENCODE_DB").filter(|v| !v.is_empty()) {
        let p = dir.join(v);
        if p.extension().is_some_and(|e| e == "db") && p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    }
    out.sort();
    out
}

struct Row {
    id: String,
    directory: String,
    parent: bool,
    updated: i64,
    count: i64,
}

fn list_table(c: &Connection, table: &str, msgs: &str) -> Vec<Row> {
    // a session created archived (`opencode run --archived`: scripts, eye's own summaries) is out
    // of OpenCode's lists, so it is out of eye's too
    let live = if has_column(c, table, "time_archived") {
        " WHERE s.time_archived IS NULL"
    } else {
        ""
    };
    let q = format!(
        "SELECT s.id, s.directory, s.parent_id, s.time_updated, (SELECT count(*) FROM {msgs} m WHERE m.session_id = s.id) FROM {table} s{live}"
    );
    let Ok(mut st) = c.prepare(&q) else {
        return vec![];
    };
    let rows = st.query_map([], |r| {
        Ok(Row {
            id: r.get(0)?,
            directory: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            parent: r
                .get::<_, Option<String>>(2)?
                .is_some_and(|p| !p.is_empty()),
            updated: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            count: r.get(4)?,
        })
    });
    rows.into_iter().flatten().flatten().collect()
}

fn project_dir(cwd: &str) -> String {
    if cwd.is_empty() {
        return "opencode".into();
    }
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn mtime(p: &Path) -> i64 {
    Stamp::of_file(p).map_or(0, |s| s.mtime)
}

/// Every session a person has, from the databases first and then the files of the older layout.
pub fn scan(dir: &Path) -> Vec<Transcript> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for db in databases(dir) {
        let Some(c) = open(&db) else { continue };
        let mut rows = list_table(&c, "session_v2", "session_message");
        rows.extend(list_table(&c, "session", "message"));
        for r in rows {
            if r.parent || !valid_session_id(&r.id) || !seen.insert(r.id.clone()) {
                continue;
            }
            out.push(Transcript {
                project_dir: project_dir(&r.directory),
                path: db_path(&db, &r.id),
                session_id: r.id,
                stamp: Stamp {
                    size: r.count,
                    mtime: r.updated,
                },
            });
        }
    }
    let sessions = dir.join("storage/session");
    for proj in std::fs::read_dir(&sessions).into_iter().flatten().flatten() {
        for f in std::fs::read_dir(proj.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = f.path();
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            if path.extension().is_none_or(|e| e != "json")
                || !valid_session_id(&id)
                || seen.contains(&id)
            {
                continue;
            }
            let Some(info) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            else {
                continue;
            };
            if info
                .get("parentID")
                .and_then(Value::as_str)
                .is_some_and(|p| !p.is_empty())
            {
                continue;
            }
            seen.insert(id.clone());
            let Some(stamp) = file_stamp(dir, &id, &path) else {
                continue;
            };
            out.push(Transcript {
                project_dir: project_dir(
                    info.get("directory").and_then(Value::as_str).unwrap_or(""),
                ),
                session_id: id,
                path,
                stamp,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The stamp of a session of the file layout: the number of its message files and the newest time
/// of them, the session file and the folder that holds them.
fn file_stamp(dir: &Path, id: &str, session_file: &Path) -> Option<Stamp> {
    let mut stamp = Stamp::of_file(session_file)?;
    stamp.size = 0;
    let msgs = dir.join("storage/message").join(id);
    stamp.mtime = stamp.mtime.max(mtime(&msgs));
    for m in std::fs::read_dir(&msgs).into_iter().flatten().flatten() {
        stamp.size += 1;
        stamp.mtime = stamp.mtime.max(mtime(&m.path()));
    }
    Some(stamp)
}

/// The data folder of a session file `<dir>/storage/session/<project>/<id>.json`.
fn data_dir_of(session_file: &Path) -> Option<&Path> {
    session_file.parent()?.parent()?.parent()?.parent()
}

/// The stamp of a session right now.
pub fn stamp(path: &Path) -> Option<Stamp> {
    if let Some((db, id)) = split_db_path(path) {
        let c = open(&db)?;
        for (t, m) in [("session_v2", "session_message"), ("session", "message")] {
            if let Some(r) = list_table(&c, t, m).into_iter().find(|r| r.id == id) {
                return Some(Stamp {
                    size: r.count,
                    mtime: r.updated,
                });
            }
        }
        return None;
    }
    let id = path.file_stem()?.to_str()?;
    file_stamp(data_dir_of(path)?, id, path)
}

/// Remove session `id` of the database or file layout that `path` names, with its sub-agent
/// sessions. Returns what was removed, as lines for people.
pub fn delete(dir: &Path, path: &Path, id: &str) -> Result<Vec<String>> {
    anyhow::ensure!(valid_session_id(id), "{id} is not an OpenCode session id");
    if let Some((db, sid)) = split_db_path(path) {
        anyhow::ensure!(
            sid == id && db.parent() == Some(dir) && db.is_file(),
            "{} is not a database of OpenCode",
            db.display()
        );
        let c = Connection::open(&db).with_context(|| format!("opening {}", db.display()))?;
        c.busy_timeout(std::time::Duration::from_millis(1_000))?;
        let tx = c.unchecked_transaction()?;
        let mut rows = 0;
        for (sessions, msgs, parts) in [
            ("session_v2", "session_message", Some("session_part")),
            ("session", "message", Some("part")),
        ] {
            if !has_table(&c, sessions) {
                continue;
            }
            let ids: Vec<String> = tx
                .prepare(&format!(
                    "SELECT id FROM {sessions} WHERE id=?1 OR parent_id=?1"
                ))?
                .query_map([id], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for sid in ids {
                for (table, col) in [
                    (Some("session_inbox"), "session_id"),
                    (parts, "session_id"),
                    (Some(msgs), "session_id"),
                    (Some(sessions), "id"),
                ] {
                    if let Some(t) = table.filter(|t| has_table(&c, t)) {
                        rows += tx.execute(&format!("DELETE FROM {t} WHERE {col}=?1"), [&sid])?;
                    }
                }
            }
        }
        tx.commit()?;
        return Ok(if rows > 0 {
            vec![format!("{rows} rows of {}", crate::util::show_path(&db))]
        } else {
            vec![]
        });
    }
    anyhow::ensure!(
        data_dir_of(path) == Some(dir)
            && path.file_stem().and_then(|s| s.to_str()) == Some(id)
            && crate::agents::is_inside(dir, path),
        "{} is not a session file of OpenCode",
        path.display()
    );
    let msgs = dir.join("storage/message").join(id);
    let mut gone = vec![];
    for m in std::fs::read_dir(&msgs).into_iter().flatten().flatten() {
        if let Some(mid) = m.path().file_stem().and_then(|s| s.to_str()) {
            if valid_session_id(mid) {
                let parts = dir.join("storage/part").join(mid);
                if parts.is_dir() {
                    std::fs::remove_dir_all(&parts)?;
                }
            }
        }
    }
    if msgs.is_dir() {
        std::fs::remove_dir_all(&msgs)?;
        gone.push(crate::util::show_path(&msgs));
    }
    if path.is_file() {
        std::fs::remove_file(path)?;
        gone.push(crate::util::show_path(path));
    }
    Ok(gone)
}

fn json_dir<T>(dir: &Path, mut each: impl FnMut(&Path, Value) -> Option<T>) -> Vec<T> {
    let mut out = vec![];
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    files.sort();
    for f in files {
        if let Some(v) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
        {
            out.extend(each(&f, v));
        }
    }
    out
}

fn load_db(db: &Path, id: &str) -> Result<Loaded> {
    let c = open(db).with_context(|| format!("opening {}", db.display()))?;
    let json = |t: String| serde_json::from_str::<Value>(&t).unwrap_or(Value::Null);
    let mut out = Loaded {
        eye_export: 1,
        id: id.to_string(),
        ..Default::default()
    };
    let session = |t: &str| {
        c.query_row(
            &format!("SELECT directory, title FROM {t} WHERE id = ?1"),
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    r.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .ok()
    };
    if has_table(&c, "session_v2") && session("session_v2").is_some() {
        let (d, t) = session("session_v2").unwrap_or_default();
        (out.directory, out.title) = (d, t);
        let mut st = c.prepare("SELECT id, type, time_created, data FROM session_message WHERE session_id = ?1 ORDER BY seq")?;
        let msgs: Vec<(String, String, i64, Value)> = st
            .query_map([id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, json(r.get(3)?)))
            })?
            .flatten()
            .collect();
        let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
        if has_table(&c, "session_part") {
            let mut st = c.prepare("SELECT message_id, data FROM session_part WHERE session_id = ?1 ORDER BY message_id, ordinal")?;
            for (m, d) in st
                .query_map([id], |r| Ok((r.get::<_, String>(0)?, json(r.get(1)?))))?
                .flatten()
            {
                parts.entry(m).or_default().push(d);
            }
        }
        out.events = from_v2(&msgs, &parts);
        out.turns = turns::from_v2(&msgs, &parts);
    } else if let Some((d, t)) = session("session") {
        (out.directory, out.title) = (d, t);
        let mut st = c.prepare("SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created, id")?;
        let msgs: Vec<(String, i64, Value)> = st
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, json(r.get(2)?))))?
            .flatten()
            .collect();
        let mut parts: HashMap<String, Vec<Value>> = HashMap::new();
        let mut st = c.prepare(
            "SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created, id",
        )?;
        for (m, d) in st
            .query_map([id], |r| Ok((r.get::<_, String>(0)?, json(r.get(1)?))))?
            .flatten()
        {
            parts.entry(m).or_default().push(d);
        }
        out.events = from_v1(&msgs, &parts);
        out.turns = turns::from_v1(&msgs, &parts);
    } else {
        anyhow::bail!("no session {id} in {}", db.display());
    }
    Ok(out)
}

fn load_files(session_file: &Path) -> Result<Loaded> {
    let info: Value = serde_json::from_str(&std::fs::read_to_string(session_file)?)?;
    let id = session_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let dir = data_dir_of(session_file).context("not a session file of OpenCode")?;
    let mut msgs: Vec<(String, i64, Value)> =
        json_dir(&dir.join("storage/message").join(&id), |_, v| {
            let created = v
                .get("time")
                .and_then(|t| t.get("created"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            Some((v.get("id")?.as_str()?.to_string(), created, v))
        });
    msgs.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    let mut parts = HashMap::new();
    for (mid, _, _) in &msgs {
        let mut ps: Vec<(i64, String, Value)> =
            json_dir(&dir.join("storage/part").join(mid), |_, v| {
                let t = v
                    .get("time")
                    .and_then(|t| t.get("start"))
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                Some((
                    t,
                    v.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    v,
                ))
            });
        ps.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        parts.insert(mid.clone(), ps.into_iter().map(|p| p.2).collect());
    }
    Ok(Loaded {
        eye_export: 1,
        id,
        directory: info
            .get("directory")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        title: info.get("title").and_then(Value::as_str).map(String::from),
        events: from_v1(&msgs, &parts),
        turns: turns::from_v1(&msgs, &parts),
        native: None,
    })
}

/// Read a session: from its database or files, or from an archived export that was put back.
pub fn load(path: &Path) -> Result<Loaded> {
    if let Some((db, id)) = split_db_path(path) {
        if !db.is_file() {
            // an export restored for reading is named like the path it replaces
            return load_export(path);
        }
        return load_db(&db, &id);
    }
    if let Ok(l) = load_export(path) {
        return Ok(l);
    }
    load_files(path)
}

fn load_export(path: &Path) -> Result<Loaded> {
    let l: Loaded = serde_json::from_slice(&std::fs::read(path)?)?;
    anyhow::ensure!(l.eye_export == 1, "not an export");
    Ok(l)
}

/// The events only, for the callers that need nothing else.
pub fn events(path: &Path) -> Result<Vec<Ev>> {
    Ok(load(path)?.events)
}

/// Is the session there: its database and row, or its file.
pub fn exists(path: &Path) -> bool {
    match split_db_path(path) {
        Some((db, _)) => db.is_file() && stamp(path).is_some(),
        None => path.is_file(),
    }
}
