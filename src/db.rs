use crate::model::{Output, Session, Stamp, Stream};
use crate::util::trim_front_bytes;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::Path;

/// The schema version this build writes; older databases are migrated on open.
pub const SCHEMA_VERSION: i64 = 12;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS session(id TEXT PRIMARY KEY, agent TEXT NOT NULL DEFAULT 'claude', project_dir TEXT, cwd TEXT, repo_root TEXT, title TEXT, title_source TEXT,
  first_prompt TEXT, started_at INTEGER, last_activity_at INTEGER, ended_at INTEGER,
  kind TEXT,
  parent_session_id TEXT, idea_id INTEGER,
  transcript_path TEXT, transcript_state TEXT,
  archive_path TEXT, git_branch TEXT, prompt_count INTEGER, reply_count INTEGER,
  last_user_text TEXT, last_assistant_text TEXT, recap TEXT, recap_at INTEGER,
  live_status TEXT, live_updated_at INTEGER, waiting_since INTEGER, waiting_note TEXT, waiting_prompt INTEGER,
  live_wait TEXT, remote_url TEXT, signals TEXT, live_loops TEXT,
  column_manual TEXT, column_manual_at INTEGER,
  ai_status TEXT, state_line TEXT, next_step TEXT, summary_at INTEGER, summary_input_hash TEXT,
  archived INTEGER NOT NULL DEFAULT 0, hidden INTEGER,
  tok_in INTEGER, tok_out INTEGER, tok_cw INTEGER, tok_cr INTEGER, tok_sub INTEGER, tok_last_id TEXT);
CREATE TABLE IF NOT EXISTS stream(id INTEGER PRIMARY KEY, session_id TEXT, kind TEXT, key TEXT, title TEXT, status TEXT, detail TEXT,
  link TEXT, sort INTEGER, updated_at INTEGER, UNIQUE(session_id, kind, key));
CREATE TABLE IF NOT EXISTS output(id INTEGER PRIMARY KEY, session_id TEXT, kind TEXT, value TEXT, url TEXT, first_seen_at INTEGER,
  UNIQUE(session_id, kind, value));
CREATE TABLE IF NOT EXISTS idea(id INTEGER PRIMARY KEY, title TEXT, notes TEXT, folder TEXT, created_at INTEGER, from_session_id TEXT,
  session_id TEXT, sort INTEGER, archived INTEGER NOT NULL DEFAULT 0, prompt TEXT, agent TEXT);
CREATE TABLE IF NOT EXISTS event(id INTEGER PRIMARY KEY, session_id TEXT, ts INTEGER, kind TEXT, payload TEXT);
CREATE TABLE IF NOT EXISTS cursor(path TEXT PRIMARY KEY, size INTEGER, mtime INTEGER, offset INTEGER, marker TEXT);
CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS jira_cand(session_id TEXT, key TEXT, ts INTEGER, PRIMARY KEY(session_id, key));
CREATE TABLE IF NOT EXISTS jira_evidence(prefix TEXT, kind TEXT, session_id TEXT, PRIMARY KEY(prefix, kind, session_id));
CREATE TABLE IF NOT EXISTS worked_hour(session_id TEXT, hour INTEGER, prompts INTEGER, events INTEGER, active_ms INTEGER,
  PRIMARY KEY(session_id, hour)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS worked_state(session_id TEXT PRIMARY KEY, last_ts INTEGER, unreadable INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS idx_stream_session ON stream(session_id);
CREATE INDEX IF NOT EXISTS idx_output_session ON output(session_id);
CREATE INDEX IF NOT EXISTS idx_event_session ON event(session_id);
CREATE VIRTUAL TABLE IF NOT EXISTS session_fts USING fts5(session_id UNINDEXED, title, prompts, replies, extra);
"#;

/// The SQLite database of sessions, streams, outputs, ideas, events and read cursors.
pub struct Db {
    pub conn: Connection,
}

/// How far something was read: the marker the adapter gave back, and the stamp it had then.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub stamp: Stamp,
    pub marker: String,
}

/// What a cursor's `offset` column holds now. The marker took its place, but a build from before
/// markers still reads that column as a byte offset. This number is past the end of any file, so
/// such a build reads the file again from the start instead of trusting an offset it did not write.
const LEGACY_OFFSET: i64 = i64::MAX;

impl Db {
    /// Open (or create) the database, readable by its owner only; SQLite gives its `-wal` and
    /// `-shm` files the same rights.
    pub fn open(path: &Path) -> Result<Db> {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        crate::files::make_private(path);
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        let _upgrading = crate::backup::before_upgrade(&conn, path, SCHEMA_VERSION);
        Self::init(conn)
    }

    /// An empty database in memory (tests).
    pub fn open_memory() -> Result<Db> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Db> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(SCHEMA).context("creating schema")?;
        let cur: i64 = self
            .meta_get("schema_version")?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // v3: manual hide override (NULL = the rule decides, 1 = hidden, 0 = shown).
        let has_hidden = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name='hidden'")?
            .exists([])?;
        if !has_hidden {
            self.add_column("session", "hidden INTEGER")?;
        }
        // v4: token counts per session (tok_sub = sub-agent total, tok_last_id = last message id counted).
        for col in [
            "tok_in",
            "tok_out",
            "tok_cw",
            "tok_cr",
            "tok_sub",
            "tok_last_id",
        ] {
            let has = self
                .conn
                .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name=?1")?
                .exists([col])?;
            if !has {
                let ty = if col == "tok_last_id" {
                    "TEXT"
                } else {
                    "INTEGER"
                };
                self.add_column("session", &format!("{col} {ty}"))?;
            }
        }
        // v5: an idea's first message to the agent, apart from its notes (which are never sent).
        let has_prompt = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('idea') WHERE name='prompt'")?
            .exists([])?;
        if !has_prompt {
            self.add_column("idea", "prompt TEXT")?;
        }
        // v6: what a waiting session asks of you, from the hooks; v7: whether a prompt stops
        // the agent mid-task until you answer it.
        for (col, ty) in [("waiting_note", "TEXT"), ("waiting_prompt", "INTEGER")] {
            let has = self
                .conn
                .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name=?1")?
                .exists([col])?;
            if !has {
                self.add_column("session", &format!("{col} {ty}"))?;
            }
        }
        // v8: which agent a session belongs to (every earlier row is Claude Code's), and which
        // agent an idea starts in (NULL = the default agent).
        let has_agent = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name='agent'")?
            .exists([])?;
        if !has_agent {
            self.add_column("session", "agent TEXT NOT NULL DEFAULT 'claude'")?;
        }
        let has_idea_agent = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('idea') WHERE name='agent'")?
            .exists([])?;
        if !has_idea_agent {
            self.add_column("idea", "agent TEXT")?;
        }
        // v9: a cursor keeps the marker an adapter hands back at its next read, where it kept a
        // byte offset. Every earlier row gets its offset as text, which is what Claude Code's
        // marker is, so nothing is read again.
        let has_marker = self
            .conn
            .prepare("SELECT 1 FROM pragma_table_info('cursor') WHERE name='marker'")?
            .exists([])?;
        if !has_marker {
            self.add_column("cursor", "marker TEXT")?;
        }
        // v10: why a live session waits, its remote control link, and the background tasks and
        // loops it runs (JSON lists, NULL when there are none).
        for col in ["live_wait", "remote_url", "signals", "live_loops"] {
            let has = self
                .conn
                .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name=?1")?
                .exists([col])?;
            if !has {
                self.add_column("session", &format!("{col} TEXT"))?;
            }
        }
        if cur < 9 {
            self.conn.execute(
                "UPDATE cursor SET marker=CAST(offset AS TEXT) WHERE marker IS NULL",
                [],
            )?;
        }
        // v11: a date to check on a session or an idea, with a note and the time you were told.
        for table in ["session", "idea"] {
            for (col, ty) in [
                ("remind_at", "INTEGER"),
                ("remind_note", "TEXT"),
                ("remind_notified_at", "INTEGER"),
            ] {
                let has = self
                    .conn
                    .prepare(&format!(
                        "SELECT 1 FROM pragma_table_info('{table}') WHERE name=?1"
                    ))?
                    .exists([col])?;
                if !has {
                    self.add_column(table, &format!("{col} {ty}"))?;
                }
            }
        }
        // v12: `worked_hour` and `worked_state` (the work timeline) are created with the schema
        // above. Nothing is read again for them: sessions without a timeline are counted by
        // `worked::backfill`, from their timestamps only.
        if cur == 1 || (cur > 0 && cur < 4) {
            // v2 keeps pending tool_use ids and Jira candidates, v4 token counts: re-read every
            // transcript once.
            self.conn.execute("DELETE FROM cursor", [])?;
        }
        if cur < SCHEMA_VERSION {
            self.meta_set("schema_version", &SCHEMA_VERSION.to_string())?;
        }
        Ok(())
    }

    /// Add a column. Another process that opened the same old file a moment earlier may have
    /// added it already; that is the state wanted, so it is not an error.
    fn add_column(&self, table: &str, decl: &str) -> Result<()> {
        match self
            .conn
            .execute(&format!("ALTER TABLE {table} ADD COLUMN {decl}"), [])
        {
            Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
            r => r
                .map(|_| ())
                .with_context(|| format!("adding {table}.{decl}")),
        }
    }

    /// A value from the `meta` table.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?)
    }

    /// Set a value in the `meta` table.
    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        meta_set(&self.conn, key, value)
    }

    /// The read cursor of a path.
    pub fn cursor_get(&self, path: &str) -> Result<Option<Cursor>> {
        cursor_get(&self.conn, path)
    }

    /// The session with exactly this id.
    pub fn session_by_id(&self, id: &str) -> Result<Option<Session>> {
        Ok(self
            .conn
            .query_row("SELECT * FROM session WHERE id=?1", [id], session_from_row)
            .optional()?)
    }

    /// Every session row.
    pub fn all_sessions(&self) -> Result<Vec<Session>> {
        let mut st = self.conn.prepare("SELECT * FROM session")?;
        let rows = st.query_map([], session_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Resolve an id or a unique prefix (at least 6 chars).
    pub fn resolve_id(&self, prefix: &str) -> Result<String> {
        let p = prefix.trim();
        if let Some(s) = self.session_by_id(p)? {
            return Ok(s.id);
        }
        anyhow::ensure!(
            p.len() >= 6,
            "session id prefix must be at least 6 characters"
        );
        let like = format!(
            "{}%",
            p.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let mut st = self
            .conn
            .prepare("SELECT id FROM session WHERE id LIKE ?1 ESCAPE '\\' LIMIT 5")?;
        let ids: Vec<String> = st
            .query_map([like], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        match ids.len() {
            0 => anyhow::bail!("no session matches '{p}'"),
            1 => Ok(ids.into_iter().next().unwrap_or_default()),
            _ => anyhow::bail!("'{p}' is ambiguous: {}", ids.join(", ")),
        }
    }

    /// The streams of one session.
    pub fn streams_for(&self, sid: &str) -> Result<Vec<Stream>> {
        let mut st = self.conn.prepare(
            "SELECT id, session_id, kind, key, title, status, detail, link, sort, updated_at
             FROM stream WHERE session_id=?1 ORDER BY CASE kind WHEN 'child' THEN 0 WHEN 'topic' THEN 1 ELSE 2 END, sort, id",
        )?;
        let rows = st.query_map([sid], |r| {
            Ok(Stream {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                key: r.get(3)?,
                title: r.get(4)?,
                status: r.get(5)?,
                detail: r.get(6)?,
                link: r.get(7)?,
                sort: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The outputs of one session.
    pub fn outputs_for(&self, sid: &str) -> Result<Vec<Output>> {
        let mut st = self.conn.prepare(&format!(
            "{OUTPUT_SELECT} WHERE o.session_id=?1 ORDER BY o.first_seen_at, o.id"
        ))?;
        let rows = st.query_map([sid], |r| {
            Ok(Output {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                value: r.get(3)?,
                url: r.get(4)?,
                first_seen_at: r.get(5)?,
                title: output_title(r)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The streams of every session, by session id.
    pub fn all_streams(&self) -> Result<std::collections::HashMap<String, Vec<Stream>>> {
        let mut st = self.conn.prepare(
            "SELECT id, session_id, kind, key, title, status, detail, link, sort, updated_at
             FROM stream ORDER BY CASE kind WHEN 'child' THEN 0 WHEN 'topic' THEN 1 ELSE 2 END, sort, id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Stream {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                key: r.get(3)?,
                title: r.get(4)?,
                status: r.get(5)?,
                detail: r.get(6)?,
                link: r.get(7)?,
                sort: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })?;
        let mut map: std::collections::HashMap<String, Vec<Stream>> = Default::default();
        for s in rows {
            let s = s?;
            map.entry(s.session_id.clone()).or_default().push(s);
        }
        Ok(map)
    }

    /// The outputs of every session, by session id.
    pub fn all_outputs(&self) -> Result<std::collections::HashMap<String, Vec<Output>>> {
        let mut st = self
            .conn
            .prepare(&format!("{OUTPUT_SELECT} ORDER BY o.first_seen_at, o.id"))?;
        let rows = st.query_map([], |r| {
            Ok(Output {
                id: r.get(0)?,
                session_id: r.get(1)?,
                kind: r.get(2)?,
                value: r.get(3)?,
                url: r.get(4)?,
                first_seen_at: r.get(5)?,
                title: output_title(r)?,
            })
        })?;
        let mut map: std::collections::HashMap<String, Vec<Output>> = Default::default();
        for o in rows {
            let o = o?;
            map.entry(o.session_id.clone()).or_default().push(o);
        }
        Ok(map)
    }

    /// Every prompt line kept for full-text search (oldest first).
    pub fn prompt_lines(&self, sid: &str) -> Result<Vec<String>> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT f.prompts FROM session_fts f JOIN session s ON s.rowid=f.rowid WHERE s.id=?1",
                [sid],
                |r| r.get(0),
            )
            .optional()?;
        Ok(text
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect())
    }

    /// SQLite's change counter; it moves when another connection writes.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// `eye hide` (Some(true)), `eye unhide` (Some(false)), or back to the rule (None).
    pub fn set_hidden(&self, id: &str, v: Option<bool>) -> Result<()> {
        self.conn.execute(
            "UPDATE session SET hidden=?2 WHERE id=?1",
            params![id, v.map(i64::from)],
        )?;
        Ok(())
    }

    /// Sessions that have at least one sub-agent or background child.
    pub fn session_ids_with_children(&self) -> Result<std::collections::HashSet<String>> {
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT session_id FROM stream WHERE kind='child'")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sessions that have at least one output.
    pub fn session_ids_with_outputs(&self) -> Result<std::collections::HashSet<String>> {
        let mut st = self
            .conn
            .prepare("SELECT DISTINCT session_id FROM output")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Run a `SELECT key, count(*) ... GROUP BY` query.
    pub fn count_group(&self, sql: &str) -> Result<Vec<(String, i64)>> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_else(|| "?".into()),
                r.get::<_, i64>(1)?,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Outputs with their stream title. A Jira key without a stored link gets
/// `https://<site>/browse/<KEY>`, the site being the one its project was seen linked on
/// (`jira_host:<PREFIX>`), else the configured one (`jira_host:*`).
const OUTPUT_SELECT: &str = "SELECT o.id, o.session_id, o.kind, o.value,
    COALESCE(o.url, CASE WHEN o.kind='jira' THEN 'https://' || NULLIF(COALESCE(
        (SELECT value FROM meta WHERE key='jira_host:'||substr(o.value, 1, instr(o.value, '-') - 1)),
        (SELECT value FROM meta WHERE key='jira_host:*')), '') || '/browse/' || o.value END) AS url,
    o.first_seen_at, s.title FROM output o
    LEFT JOIN stream s ON s.session_id=o.session_id AND s.kind='output' AND s.key=o.kind||':'||o.value";

/// Set a value in the `meta` table of `conn` (also inside a transaction).
pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// The read cursor of a path (also inside a transaction). When a build from before markers ran
/// after this one, it left a real number in `offset`. That is the newest word on how far the file
/// was read, and as a byte offset it is the marker of a file read by byte offset.
pub fn cursor_get(conn: &Connection, path: &str) -> Result<Option<Cursor>> {
    Ok(conn
        .query_row(
            "SELECT size, mtime,
                    CASE WHEN offset IS NOT NULL AND offset <> ?2 THEN CAST(offset AS TEXT) ELSE marker END
             FROM cursor WHERE path=?1",
            params![path, LEGACY_OFFSET],
            |r| {
                Ok(Cursor {
                    stamp: Stamp {
                        size: r.get(0)?,
                        mtime: r.get(1)?,
                    },
                    marker: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                })
            },
        )
        .optional()?)
}

/// Record how far a path was read: the stamp it has now and the marker the adapter returned.
pub fn cursor_set(conn: &Connection, path: &str, stamp: Stamp, marker: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO cursor(path,size,mtime,offset,marker) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(path) DO UPDATE SET size=excluded.size, mtime=excluded.mtime,
           offset=excluded.offset, marker=excluded.marker",
        params![path, stamp.size, stamp.mtime, LEGACY_OFFSET, marker],
    )?;
    Ok(())
}

/// The stream title of an output row (column 6, value in column 3). `add_output` stores the
/// value itself when no title is known, so that counts as none.
fn output_title(r: &Row) -> rusqlite::Result<Option<String>> {
    let value: String = r.get(3)?;
    Ok(r.get::<_, Option<String>>(6)?
        .filter(|t| !t.trim().is_empty() && *t != value))
}

/// A session from a `SELECT * FROM session` row.
pub fn session_from_row(r: &Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: r.get("id")?,
        agent: r.get("agent")?,
        project_dir: r.get("project_dir")?,
        cwd: r.get("cwd")?,
        repo_root: r.get("repo_root")?,
        title: r.get("title")?,
        title_source: r.get("title_source")?,
        first_prompt: r.get("first_prompt")?,
        started_at: r.get("started_at")?,
        last_activity_at: r.get("last_activity_at")?,
        ended_at: r.get("ended_at")?,
        kind: r.get("kind")?,
        parent_session_id: r.get("parent_session_id")?,
        idea_id: r.get("idea_id")?,
        transcript_path: r.get("transcript_path")?,
        transcript_state: r.get("transcript_state")?,
        archive_path: r.get("archive_path")?,
        git_branch: r.get("git_branch")?,
        prompt_count: r.get::<_, Option<i64>>("prompt_count")?.unwrap_or(0),
        reply_count: r.get::<_, Option<i64>>("reply_count")?.unwrap_or(0),
        last_user_text: r.get("last_user_text")?,
        last_assistant_text: r.get("last_assistant_text")?,
        recap: r.get("recap")?,
        recap_at: r.get("recap_at")?,
        live_status: r.get("live_status")?,
        live_updated_at: r.get("live_updated_at")?,
        waiting_since: r.get("waiting_since")?,
        waiting_note: r.get("waiting_note")?,
        waiting_prompt: r.get::<_, Option<i64>>("waiting_prompt")?.unwrap_or(0) != 0,
        live_wait: r.get("live_wait")?,
        remote_url: r.get("remote_url")?,
        signals: crate::activity::from_column(r.get::<_, Option<String>>("signals")?.as_deref()),
        live_loops: crate::activity::from_column(
            r.get::<_, Option<String>>("live_loops")?.as_deref(),
        ),
        column_manual: r.get("column_manual")?,
        column_manual_at: r.get("column_manual_at")?,
        ai_status: r.get("ai_status")?,
        state_line: r.get("state_line")?,
        next_step: r.get("next_step")?,
        summary_at: r.get("summary_at")?,
        summary_input_hash: r.get("summary_input_hash")?,
        archived: r.get::<_, i64>("archived")? != 0,
        hidden: r.get::<_, Option<i64>>("hidden")?.map(|v| v != 0),
        remind_at: r.get("remind_at")?,
        remind_note: r.get("remind_note")?,
        remind_notified_at: r.get("remind_notified_at")?,
        tokens: match r.get::<_, Option<i64>>("tok_in")? {
            Some(input) => Some(crate::model::Tokens {
                input,
                output: r.get::<_, Option<i64>>("tok_out")?.unwrap_or(0),
                cache_write: r.get::<_, Option<i64>>("tok_cw")?.unwrap_or(0),
                cache_read: r.get::<_, Option<i64>>("tok_cr")?.unwrap_or(0),
                subagents_total: r.get::<_, Option<i64>>("tok_sub")?.unwrap_or(0),
            }),
            None => None,
        },
    })
}

/// Rewrite the FTS row of a session. The FTS rowid is the session rowid, so lookups never scan.
/// `add_*` text is appended to what the row already holds (unless `reset`), then capped to the
/// most recent bytes. Title, recap, state, next step and outputs are rebuilt each time.
pub fn fts_write(
    conn: &Connection,
    sid: &str,
    add_prompts: &str,
    add_replies: &str,
    reset: bool,
    prompts_cap: usize,
    replies_cap: usize,
) -> Result<()> {
    let Some(rowid) = conn
        .query_row("SELECT rowid FROM session WHERE id=?1", [sid], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
    else {
        return Ok(());
    };
    let (mut prompts, mut replies) = if reset {
        (String::new(), String::new())
    } else {
        conn.query_row(
            "SELECT prompts, replies FROM session_fts WHERE rowid=?1",
            [rowid],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
        .unwrap_or_default()
    };
    prompts.push_str(add_prompts);
    replies.push_str(add_replies);
    trim_front_bytes(&mut prompts, prompts_cap);
    trim_front_bytes(&mut replies, replies_cap);
    let (title, recap, state, next): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT title, recap, state_line, next_step FROM session WHERE id=?1",
        [sid],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let mut extra = String::new();
    for p in [recap, state, next].into_iter().flatten() {
        extra.push_str(&p);
        extra.push('\n');
    }
    let mut st = conn.prepare_cached(
        "SELECT o.kind, o.value, s.title FROM output o
         LEFT JOIN stream s ON s.session_id=o.session_id AND s.kind='output' AND s.key=o.kind||':'||o.value
         WHERE o.session_id=?1",
    )?;
    let rows = st.query_map([sid], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (kind, value, title) = row?;
        extra.push_str(&kind);
        extra.push(' ');
        extra.push_str(&value);
        if let Some(t) = title.filter(|t| *t != value) {
            extra.push(' ');
            extra.push_str(&t);
        }
        extra.push('\n');
    }
    conn.execute("DELETE FROM session_fts WHERE rowid=?1", [rowid])?;
    conn.execute(
        "INSERT INTO session_fts(rowid, session_id, title, prompts, replies, extra) VALUES(?1,?2,?3,?4,?5,?6)",
        params![rowid, sid, title.unwrap_or_default(), prompts, replies, extra],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
