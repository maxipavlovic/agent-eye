//! Where agy keeps its files (`~/.gemini/antigravity-cli`) and how a conversation is found:
//! `conversations/<id>.db` is the conversation, `brain/<id>/` its folder, and
//! `brain/<id>/.system_generated/logs/transcript_full.jsonl` the plain JSON log of its steps that
//! eye reads. Titles and folders are in `conversation_summaries.db`, prompts in `history.jsonl`.

use crate::agents::Transcript;
use crate::model::Stamp;
use crate::util::{iso_to_ms, valid_session_id};
use rusqlite::{Connection, OpenFlags};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

pub const CONVERSATIONS: &str = "conversations";
pub const BRAIN: &str = "brain";
const LOG_REL: &str = ".system_generated/logs/transcript_full.jsonl";
/// The database snapshot inside an archived copy of a conversation's folder.
pub const SNAPSHOT: &str = "eye-conversation.db";
/// The folders eye runs `agy` in for summaries start with this; those conversations are not sessions.
pub const ASK_PREFIX: &str = "eye-ask-";

/// What `conversation_summaries.db` says about a conversation.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub title: Option<String>,
    pub folder: Option<String>,
    /// A conversation of a sub-agent: shown nowhere on its own.
    pub nested: bool,
    pub modified: i64,
    /// The conversation this one is a sub-agent of (`parent_conversation_id`).
    pub parent: Option<String>,
    /// `source`: what agy says started the conversation. agy 1.5.14 writes it empty.
    pub source: String,
    /// `agent_name`: the kind of sub-agent (`self`), empty for a conversation of a person.
    pub agent: String,
    /// `status` is `CASCADE_RUN_STATUS_RUNNING` (or `BUSY`): a turn is running.
    pub running: bool,
}

pub fn log_of(folder: &Path) -> PathBuf {
    folder.join(LOG_REL)
}

/// `/a b#c` as a path in a `file:` URI.
fn uri_encode(p: &Path) -> String {
    let mut out = String::new();
    for b in p.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/._-~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Open a database of agy for reading. agy's databases are in WAL mode, and SQLite cannot open one
/// read-only when no process has it open (its `-wal` and `-shm` files are gone): the first query
/// fails with "unable to open database file". Then nobody is writing, and the file is read as it
/// is (`immutable=1`).
pub fn open_ro(path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let probe = |c: &Connection| {
        c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })
        .is_ok()
    };
    if let Ok(c) = Connection::open_with_flags(path, flags) {
        let _ = c.busy_timeout(std::time::Duration::from_millis(300));
        if probe(&c) {
            return Some(c);
        }
    }
    let uri = format!("file:{}?immutable=1", uri_encode(path));
    let c = Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI).ok()?;
    probe(&c).then_some(c)
}

/// `file:///a%20b` as `/a b`.
pub fn uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let b = rest.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        let hex = |x: u8| (x as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    s.starts_with('/').then_some(s)
}

/// Titles and folders of every conversation. Empty when the database is missing or unreadable.
pub fn summaries(dir: &Path) -> HashMap<String, Summary> {
    let mut out = HashMap::new();
    let Some(c) = open_ro(&dir.join("conversation_summaries.db")) else {
        return out;
    };
    // `source`, `agent_name`, `status` and `killed` may be missing in an older agy
    let Ok(mut st) = c
        .prepare(
            "SELECT conversation_id, title, workspace_uris, parent_conversation_id, \
             last_modified_time, source, agent_name, status FROM conversation_summaries",
        )
        .or_else(|_| {
            c.prepare(
                "SELECT conversation_id, title, workspace_uris, parent_conversation_id, \
                 last_modified_time, '', '', '' FROM conversation_summaries",
            )
        })
    else {
        return out;
    };
    let rows = st.query_map([], |r| {
        let text = |i: usize| {
            r.get::<_, Option<String>>(i)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        Ok((
            text(0),
            text(1),
            text(2),
            text(3),
            text(4),
            text(5),
            text(6),
            text(7),
        ))
    });
    for (id, title, uris, parent, modified, source, agent, status) in
        rows.into_iter().flatten().flatten()
    {
        let folder = serde_json::from_str::<Vec<String>>(&uris)
            .ok()
            .and_then(|v| v.iter().find_map(|u| uri_path(u)));
        let modified = iso_to_ms(&modified.replacen(' ', "T", 1)).unwrap_or(0);
        out.insert(
            id,
            Summary {
                title: Some(title.trim().to_string()).filter(|t| !t.is_empty()),
                folder,
                nested: !parent.is_empty(),
                modified,
                parent: Some(parent).filter(|p| !p.is_empty()),
                source: source.trim().to_string(),
                agent: agent.trim().to_string(),
                running: status.ends_with("_RUNNING") || status.ends_with("_BUSY"),
            },
        );
    }
    out
}

/// The folder each conversation was last typed in, from `history.jsonl`.
fn history_folders(dir: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(dir.join("history.jsonl")) else {
        return out;
    };
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else {
            continue;
        };
        if let (Some(id), Some(ws)) = (
            v.get("conversationId").and_then(|x| x.as_str()),
            v.get("workspace").and_then(|x| x.as_str()),
        ) {
            out.insert(id.to_string(), ws.to_string());
        }
    }
    out
}

/// Titles and folders as `summaries` has them, with the folder from `history.jsonl` for a
/// conversation whose row names none (agy 1.5.14 leaves it empty for a conversation of the
/// terminal).
pub fn summaries_with_folders(dir: &Path) -> HashMap<String, Summary> {
    let mut sums = summaries(dir);
    for (id, folder) in history_folders(dir) {
        if let Some(s) = sums.get_mut(&id) {
            if s.folder.is_none() {
                s.folder = Some(folder);
            }
        }
    }
    sums
}

/// The sub-agent conversations below `parent` (children, grandchildren, ...), oldest first.
pub fn descendants(sums: &HashMap<String, Summary>, parent: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut todo = vec![parent.to_string()];
    while let Some(p) = todo.pop() {
        let mut kids: Vec<&String> = sums
            .iter()
            .filter(|(id, s)| s.parent.as_deref() == Some(p.as_str()) && !out.contains(id))
            .map(|(id, _)| id)
            .collect();
        kids.sort();
        for k in kids {
            out.push(k.clone());
            todo.push(k.clone());
        }
    }
    out
}

/// The role and the kind the parent gave sub-agent `child` (`brain/<parent>/.system_generated/
/// subagents/<child>.json`: `subagentDescriptor.role` and `.typeName`).
pub fn subagent_manifest(dir: &Path, parent: &str, child: &str) -> Option<(String, String)> {
    let p = dir
        .join(BRAIN)
        .join(parent)
        .join(".system_generated/subagents")
        .join(format!("{child}.json"));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).ok()?).ok()?;
    let d = v.get("subagentDescriptor")?;
    let t = |k: &str| {
        d.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    Some((t("role"), t("typeName")))
}

/// The database of the conversation in `folder`: the snapshot of an archived copy, else the live
/// one next to `brain/`.
pub fn db_of(folder: &Path) -> Option<PathBuf> {
    let snap = folder.join(SNAPSHOT);
    if snap.is_file() {
        return Some(snap);
    }
    let id = folder.file_name()?.to_str()?;
    let live = folder
        .parent()?
        .parent()?
        .join(CONVERSATIONS)
        .join(format!("{id}.db"));
    live.is_file().then_some(live)
}

/// The change stamp of a conversation: the size of its log, and the newest time of the log, the
/// database (and its write-ahead log) and `extra` (the summaries' last change).
pub fn stamp(folder: &Path, extra: i64) -> Option<Stamp> {
    let log = Stamp::of_file(&log_of(folder))?;
    let mut mtime = log.mtime.max(extra);
    if let Some(db) = db_of(folder) {
        let wal = PathBuf::from(format!("{}-wal", db.display()));
        for p in [db, wal] {
            if let Some(s) = Stamp::of_file(&p) {
                mtime = mtime.max(s.mtime);
            }
        }
    }
    Some(Stamp {
        size: log.size,
        mtime,
    })
}

/// Remove conversation `id` (its folder is `folder`) and the sub-agent conversations below it: the
/// folder, the database and its `-shm` and `-wal` files, the presence lock, the history lines, the
/// cache entries and the row in `conversation_summaries.db`.
pub fn delete(dir: &Path, folder: &Path, id: &str) -> anyhow::Result<Vec<String>> {
    let kids = descendants(&summaries(dir), id);
    let mut gone = delete_one(dir, folder, id)?;
    for k in kids {
        gone.extend(delete_one(dir, &dir.join(BRAIN).join(&k), &k)?);
    }
    Ok(gone)
}

fn delete_one(dir: &Path, folder: &Path, id: &str) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        crate::agents::is_inside(&dir.join(BRAIN), folder)
            && valid_session_id(id)
            && folder.file_name().and_then(|n| n.to_str()) == Some(id),
        "{} is not a conversation of agy",
        folder.display()
    );
    let mut gone = vec![];
    if std::fs::symlink_metadata(folder).is_ok() {
        std::fs::remove_dir_all(folder)?;
        gone.push(crate::util::show_path(folder));
    }
    let db = dir.join(CONVERSATIONS).join(format!("{id}.db"));
    for p in [
        db.clone(),
        PathBuf::from(format!("{}-shm", db.display())),
        PathBuf::from(format!("{}-wal", db.display())),
    ] {
        if p.is_file() {
            std::fs::remove_file(&p)?;
            gone.push(crate::util::show_path(&p));
        }
    }
    for p in [
        dir.join("presence").join(format!("{id}.lock")),
        dir.join("annotations").join(format!("{id}.pbtxt")),
    ] {
        if p.is_file() {
            std::fs::remove_file(&p)?;
            gone.push(crate::util::show_path(&p));
        }
    }
    let lines = forget_history(dir, id)?;
    if lines > 0 {
        gone.push(format!("{lines} line(s) of history.jsonl"));
    }
    for f in scrub_caches(dir, id)? {
        gone.push(f);
    }
    let sums = dir.join("conversation_summaries.db");
    if sums.is_file() {
        let c = Connection::open(&sums)?;
        c.busy_timeout(std::time::Duration::from_millis(1_000))?;
        // zero the deleted row's bytes too: the title and the folder are in it
        c.query_row("PRAGMA secure_delete = ON", [], |_| Ok(()))?;
        if c.execute(
            "DELETE FROM conversation_summaries WHERE conversation_id=?1",
            [id],
        )? > 0
        {
            gone.push("its row in conversation_summaries.db".to_string());
        }
    }
    Ok(gone)
}

/// Remove the lines of `history.jsonl` typed in conversation `id`, so nothing of it stays in agy's
/// up-arrow history. Every other line is kept byte for byte. Returns how many lines went.
pub fn forget_history(dir: &Path, id: &str) -> anyhow::Result<usize> {
    let path = dir.join("history.jsonl");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(0);
    };
    let (mut kept, mut gone) = (String::with_capacity(text.len()), 0);
    for line in text.split_inclusive('\n') {
        let mine = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .is_some_and(|v| v.get("conversationId").and_then(|c| c.as_str()) == Some(id));
        if mine {
            gone += 1;
        } else {
            kept.push_str(line);
        }
    }
    if gone > 0 {
        crate::files::write_file(&path, kept.as_bytes(), 0o600)?;
    }
    Ok(gone)
}

/// Take conversation `id` out of agy's caches: `cache/last_conversations.json` (folder to the
/// conversation `--continue` opens there) and `cache/conversation_metadata.json`. Returns what
/// was changed, as lines for people.
pub fn scrub_caches(dir: &Path, id: &str) -> anyhow::Result<Vec<String>> {
    use serde_json::Value;
    let mut gone = vec![];
    let edit = |name: &str, f: &dyn Fn(&mut Value) -> bool| -> anyhow::Result<bool> {
        let path = dir.join("cache").join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Ok(false);
        };
        let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
            return Ok(false);
        };
        if !f(&mut v) {
            return Ok(false);
        }
        let mut out = serde_json::to_string_pretty(&v)?;
        if text.ends_with('\n') {
            out.push('\n');
        }
        crate::files::write_file(&path, out.as_bytes(), 0o600)?;
        Ok(true)
    };
    if edit("last_conversations.json", &|v| {
        let Some(m) = v.as_object_mut() else {
            return false;
        };
        let before = m.len();
        m.retain(|_, c| c.as_str() != Some(id));
        m.len() != before
    })? {
        gone.push("its entry in cache/last_conversations.json".to_string());
    }
    if edit("conversation_metadata.json", &|v| {
        v.get_mut("conversations")
            .and_then(Value::as_object_mut)
            .is_some_and(|m| m.shift_remove(id).is_some())
    })? {
        gone.push("its entry in cache/conversation_metadata.json".to_string());
    }
    Ok(gone)
}

/// The name of the archive folder for a conversation in `folder`: its project, Claude Code style.
fn project_dir(cwd: Option<&str>) -> String {
    match cwd {
        Some(c) if !c.is_empty() => c
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect(),
        _ => "agy".into(),
    }
}

/// The conversations a person has: with a log, not a sub-agent's, not one eye started for a summary.
pub fn scan(dir: &Path) -> (Vec<Transcript>, HashMap<String, Summary>) {
    let Ok(rd) = std::fs::read_dir(dir.join(CONVERSATIONS)) else {
        return (vec![], HashMap::new());
    };
    let mut sums = summaries(dir);
    let hist = history_folders(dir);
    let mut out = vec![];
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".db").filter(|i| valid_session_id(i)) else {
            continue;
        };
        let folder = dir.join(BRAIN).join(id);
        let s = sums.entry(id.to_string()).or_default();
        if s.folder.is_none() {
            s.folder = hist.get(id).cloned();
        }
        let ask = s.folder.as_deref().is_some_and(|f| {
            Path::new(f)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(ASK_PREFIX))
        });
        if s.nested || ask {
            continue;
        }
        let Some(stamp) = stamp(&folder, s.modified) else {
            continue;
        };
        out.push(Transcript {
            project_dir: project_dir(s.folder.as_deref()),
            session_id: id.to_string(),
            path: folder,
            stamp,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    (out, sums)
}

/// A relative path made only of plain names (what an archived copy may hold).
pub fn plain_relative(p: &Path) -> bool {
    p.components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(
            uri_path("file:///Users/x/a%20b").as_deref(),
            Some("/Users/x/a b")
        );
        assert_eq!(uri_path("file:///x%2").as_deref(), Some("/x%2"));
        assert_eq!(uri_path("https://x"), None);
        assert_eq!(uri_path("file://rel"), None);
    }

    #[test]
    fn the_project_dir_is_the_folder_with_dashes() {
        assert_eq!(project_dir(Some("/Users/a b/c")), "-Users-a-b-c");
        assert_eq!(project_dir(None), "agy");
    }
}
