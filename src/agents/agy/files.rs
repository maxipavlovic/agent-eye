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
}

pub fn log_of(folder: &Path) -> PathBuf {
    folder.join(LOG_REL)
}

pub fn open_ro(path: &Path) -> Option<Connection> {
    let c = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    c.busy_timeout(std::time::Duration::from_millis(300)).ok()?;
    Some(c)
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
    let Ok(mut st) = c.prepare(
        "SELECT conversation_id, title, workspace_uris, parent_conversation_id, last_modified_time \
         FROM conversation_summaries",
    ) else {
        return out;
    };
    let rows = st.query_map([], |r| {
        let text = |i: usize| {
            r.get::<_, Option<String>>(i)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        Ok((text(0), text(1), text(2), text(3), text(4)))
    });
    for (id, title, uris, parent, modified) in rows.into_iter().flatten().flatten() {
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

/// Remove conversation `id` (its folder is `folder`): the folder, the database and its `-shm` and
/// `-wal` files, and its row in `conversation_summaries.db`.
pub fn delete(dir: &Path, folder: &Path, id: &str) -> anyhow::Result<Vec<String>> {
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
    let sums = dir.join("conversation_summaries.db");
    if sums.is_file() {
        let c = Connection::open(&sums)?;
        c.busy_timeout(std::time::Duration::from_millis(1_000))?;
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
