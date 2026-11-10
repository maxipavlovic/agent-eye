//! A third agent for the tests, whose sessions are folders with a SQLite database in them, not
//! files of lines. It shows that an adapter owns how its sessions are read, stamped and archived.
//!
//! Its files: `<dir>/sessions/<project>/<session>/` holds `meta.json` (`{"cwd": "...", "epoch": N}`)
//! and `session.db`, whose `messages` table has one row per message. The agent rewrites a whole
//! session (a compaction) by replacing every row and raising `epoch`. The read marker is
//! `<epoch>:<last row id>`; the change stamp is the total size and the newest modification time of
//! the files in the folder. A copy in the archive is a folder with each file compressed, and the
//! database in it is a snapshot made with `VACUUM INTO`.

use agent_eye::agents::{
    self, Agent, AgentRef, Delta, Detection, Invocation, ParseOpts, Stamp, StartPlan, StartRequest,
    TailItem, Transcript, PROMPTS_CAP, REPLIES_CAP,
};
use agent_eye::archive::{compress_file, decompress_file};
use agent_eye::config::AgentCfg;
use agent_eye::conversation::Message;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const ID: &str = "fakedb";

#[derive(Debug)]
pub struct FolderAgent {
    dir: PathBuf,
}

/// Put the agent in the registry, so a config can enable it.
pub fn register() {
    agents::register(ID, |cfg: &AgentCfg| -> AgentRef {
        Arc::new(FolderAgent {
            dir: PathBuf::from(cfg.dir.clone().unwrap_or_default()),
        })
    });
}

/// What the agent itself does to its files, for the tests: start a session, add messages, rewrite
/// the whole session.
pub mod agent_writes {
    use super::*;

    fn folder(dir: &Path, project: &str, id: &str) -> PathBuf {
        dir.join("sessions").join(project).join(id)
    }

    /// Each write waits a moment first, so the modification time of the files moves (the stamp is
    /// size and time, and a database often stays the same size when a row is added).
    fn pause() {
        std::thread::sleep(Duration::from_millis(5));
    }

    fn write_meta(folder: &Path, cwd: &str, epoch: i64) {
        std::fs::write(
            folder.join("meta.json"),
            json!({"cwd": cwd, "epoch": epoch}).to_string(),
        )
        .unwrap();
    }

    fn insert(conn: &Connection, rows: &[(&str, i64, &str)]) {
        for (role, ts, text) in rows {
            conn.execute(
                "INSERT INTO messages(role, ts, text) VALUES(?1, ?2, ?3)",
                (role, ts, text),
            )
            .unwrap();
        }
    }

    /// A new session with its first messages (`u` is the user, `a` the agent) at epoch 1.
    pub fn start(dir: &Path, project: &str, id: &str, cwd: &str, rows: &[(&str, i64, &str)]) {
        let f = folder(dir, project, id);
        std::fs::create_dir_all(&f).unwrap();
        let conn = Connection::open(f.join("session.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE messages(id INTEGER PRIMARY KEY AUTOINCREMENT, role TEXT NOT NULL,
                                   ts INTEGER NOT NULL, text TEXT NOT NULL);",
        )
        .unwrap();
        insert(&conn, rows);
        write_meta(&f, cwd, 1);
    }

    /// More messages at the end of a session.
    pub fn add(dir: &Path, project: &str, id: &str, rows: &[(&str, i64, &str)]) {
        pause();
        let f = folder(dir, project, id);
        insert(&Connection::open(f.join("session.db")).unwrap(), rows);
    }

    /// The whole session written again with other messages: new epoch, every row replaced.
    pub fn rewrite(dir: &Path, project: &str, id: &str, rows: &[(&str, i64, &str)]) {
        pause();
        let f = folder(dir, project, id);
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(f.join("meta.json")).unwrap()).unwrap();
        let conn = Connection::open(f.join("session.db")).unwrap();
        conn.execute_batch("DELETE FROM messages; DELETE FROM sqlite_sequence;")
            .unwrap();
        insert(&conn, rows);
        write_meta(
            &f,
            meta["cwd"].as_str().unwrap(),
            meta["epoch"].as_i64().unwrap() + 1,
        );
    }

    /// The folder of a session.
    pub fn path(dir: &Path, project: &str, id: &str) -> PathBuf {
        folder(dir, project, id)
    }
}

fn open(folder: &Path) -> Result<Connection> {
    let db = folder.join("session.db");
    Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", db.display()))
}

fn meta(folder: &Path) -> Result<(String, i64)> {
    let text = std::fs::read_to_string(folder.join("meta.json"))?;
    let v: Value = serde_json::from_str(&text)?;
    Ok((
        v["cwd"].as_str().unwrap_or_default().to_string(),
        v["epoch"].as_i64().unwrap_or(0),
    ))
}

/// Messages after row `after`, oldest first: (id, role, ts, text).
fn rows(conn: &Connection, after: i64) -> Result<Vec<(i64, String, i64, String)>> {
    let mut st =
        conn.prepare("SELECT id, role, ts, text FROM messages WHERE id > ?1 ORDER BY id")?;
    let rows = st.query_map([after], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn messages(folder: &Path) -> Result<Vec<Message>> {
    Ok(rows(&open(folder)?, 0)?
        .into_iter()
        .map(|(_, role, ts, text)| {
            let role = if role == "u" { "user" } else { "assistant" };
            Message::new(role, &text, Some(ts), 0)
        })
        .collect())
}

/// The total size and the newest modification time of the files in a session folder.
fn folder_stamp(folder: &Path) -> Option<Stamp> {
    let mut stamp = Stamp { size: 0, mtime: 0 };
    for e in std::fs::read_dir(folder).ok()?.flatten() {
        let s = Stamp::of_metadata(&e.metadata().ok()?);
        stamp.size += s.size;
        stamp.mtime = stamp.mtime.max(s.mtime);
    }
    Some(stamp)
}

fn parse_marker(marker: &str) -> Option<(i64, i64)> {
    let (epoch, last) = marker.split_once(':')?;
    Some((epoch.parse().ok()?, last.parse().ok()?))
}

impl Agent for FolderAgent {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Fake DB Agent"
    }
    fn short_name(&self) -> &'static str {
        "Dbbot"
    }
    fn detect(&self) -> Detection {
        Detection {
            installed: self.dir.is_dir(),
            detail: format!("looked in {}", self.dir.display()),
        }
    }

    fn transcripts(&self) -> Vec<Transcript> {
        let mut out = vec![];
        let Ok(projects) = std::fs::read_dir(self.dir.join("sessions")) else {
            return out;
        };
        for project in projects.flatten().filter(|p| p.path().is_dir()) {
            for s in std::fs::read_dir(project.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let (path, Some(id)) = (s.path(), s.file_name().to_str().map(String::from)) else {
                    continue;
                };
                let Some(stamp) = folder_stamp(&path) else {
                    continue;
                };
                out.push(Transcript {
                    project_dir: project.file_name().to_string_lossy().into_owned(),
                    session_id: id,
                    path,
                    stamp,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn parse_transcript(&self, t: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        let (cwd, epoch) = meta(&t.path)?;
        let conn = open(&t.path)?;
        let newest: i64 = conn.query_row("SELECT coalesce(max(id), 0) FROM messages", [], |r| {
            r.get(0)
        })?;
        // Go on after the last row read, unless the agent rewrote the session (another epoch) or
        // has fewer rows than before: then read everything again.
        let after = opts
            .since
            .and_then(|s| parse_marker(s.marker))
            .filter(|(e, last)| *e == epoch && *last <= newest)
            .map(|(_, last)| last);
        let mut d = Delta {
            marker: format!("{epoch}:{newest}"),
            replaces: after.is_none(),
            session_id: Some(t.session_id.clone()),
            cwd: Some(cwd),
            ..Delta::default()
        };
        for (_, role, ts, text) in rows(&conn, after.unwrap_or(0))? {
            d.lines += 1;
            // this agent counts rows, not bytes
            d.bytes_read += 1;
            d.first_ts = Some(d.first_ts.map_or(ts, |t| t.min(ts)));
            d.last_ts = Some(d.last_ts.map_or(ts, |t| t.max(ts)));
            if role == "u" {
                d.prompt_count += 1;
                d.first_prompt.get_or_insert_with(|| text.clone());
                d.last_user_text = Some(text.clone());
                d.prompts_text.push_str(&text);
                d.prompts_text.push('\n');
            } else {
                d.reply_count += 1;
                d.last_assistant_text = Some(text.clone());
                d.replies_text.push_str(&text);
                d.replies_text.push('\n');
            }
        }
        d.prompts_text.truncate(PROMPTS_CAP);
        d.replies_text.truncate(REPLIES_CAP);
        Ok(d)
    }

    fn owns_transcript(&self, path: &Path) -> bool {
        path.starts_with(self.dir.join("sessions"))
    }

    fn transcript_stamp(&self, path: &Path) -> Option<Stamp> {
        folder_stamp(path)
    }
    fn archive_name(&self, session_id: &str) -> String {
        session_id.to_string()
    }
    fn archive(&self, path: &Path, copy: &Path) -> Result<()> {
        std::fs::create_dir(copy)?;
        compress_file(&path.join("meta.json"), &copy.join("meta.json.zst"))?;
        // a consistent snapshot of the database, also while the agent writes to it
        let snapshot = copy.join("session.db");
        open(path)?.execute("VACUUM INTO ?1", [snapshot.to_string_lossy()])?;
        compress_file(&snapshot, &copy.join("session.db.zst"))?;
        std::fs::remove_file(snapshot)?;
        Ok(())
    }
    fn unarchive(&self, copy: &Path, dest: &Path) -> Result<()> {
        std::fs::create_dir(dest)?;
        for name in ["meta.json", "session.db"] {
            decompress_file(&copy.join(format!("{name}.zst")), &dest.join(name))?;
        }
        Ok(())
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<Message>> {
        messages(path)
    }
    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        Ok(messages(path)?
            .into_iter()
            .map(|m| TailItem::Text {
                role: m.role,
                text: m.text,
            })
            .collect())
    }

    fn executable(&self) -> String {
        "dbbot".into()
    }
    fn resume(&self, session_id: &str) -> Invocation {
        Invocation::new("dbbot", &["resume", session_id])
    }
    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        Ok(StartPlan {
            invocation: Invocation {
                program: "dbbot".into(),
                args: vec!["new".into(), req.title.into()],
            },
            session_id: None,
        })
    }
}
