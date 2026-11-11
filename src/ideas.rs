use crate::agents::{self, AgentRef, Agents, Invocation, StartRequest};
use crate::config::Config;
use crate::db::{meta_set, Db};
use crate::folder::{self, FolderMissing, FolderState};
use crate::model::Idea;
use crate::paths::Paths;
use crate::resume;
use crate::util::now_ms;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Idea> {
    Ok(Idea {
        id: r.get("id")?,
        agent: r.get("agent")?,
        title: r.get::<_, Option<String>>("title")?.unwrap_or_default(),
        notes: r.get("notes")?,
        prompt: r.get("prompt")?,
        folder: r.get("folder")?,
        created_at: r.get("created_at")?,
        from_session_id: r.get("from_session_id")?,
        session_id: r.get("session_id")?,
        sort: r.get("sort")?,
        archived: r.get::<_, i64>("archived")? != 0,
        remind_at: r.get("remind_at")?,
        remind_note: r.get("remind_note")?,
        remind_notified_at: r.get("remind_notified_at")?,
    })
}

fn clean(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|v| !v.is_empty())
}

/// A new idea.
#[derive(Debug, Clone, Default)]
pub struct NewIdea<'a> {
    pub title: &'a str,
    /// Yours; never sent to the agent.
    pub notes: Option<&'a str>,
    /// The first message of the session the idea starts.
    pub prompt: Option<&'a str>,
    pub folder: Option<&'a str>,
    /// The session the idea came from.
    pub from_session: Option<&'a str>,
    /// The agent to start it in; None is the default agent.
    pub agent: Option<&'a str>,
}

/// Park a new idea.
pub fn add(db: &Db, n: &NewIdea) -> Result<Idea> {
    let title = n.title.trim();
    anyhow::ensure!(!title.is_empty(), "an idea needs a title");
    let sort: i64 = db
        .conn
        .query_row("SELECT COALESCE(MAX(sort), 0) + 1 FROM idea", [], |r| {
            r.get(0)
        })?;
    db.conn.execute(
        "INSERT INTO idea(title, notes, prompt, folder, created_at, from_session_id, sort, agent) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            title,
            clean(n.notes),
            clean(n.prompt),
            clean(n.folder),
            now_ms(),
            n.from_session,
            sort,
            clean(n.agent)
        ],
    )?;
    get(db, db.conn.last_insert_rowid())?.context("idea vanished")
}

/// What `edit` changes: a field left None stays as it is, an empty one is cleared.
#[derive(Debug, Clone, Default)]
pub struct Edit {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub prompt: Option<String>,
    pub folder: Option<String>,
    pub agent: Option<String>,
}

/// Change an idea; see [`Edit`].
pub fn edit(db: &Db, id: i64, e: &Edit) -> Result<Idea> {
    let cur = get(db, id)?.context("no such idea")?;
    let title = match e.title.as_deref().map(str::trim) {
        Some("") => anyhow::bail!("an idea needs a title"),
        Some(t) => t.to_string(),
        None => cur.title,
    };
    let pick = |new: &Option<String>, old: Option<String>| match new {
        Some(v) => clean(Some(v)).map(str::to_string),
        None => old,
    };
    db.conn.execute(
        "UPDATE idea SET title=?2, notes=?3, prompt=?4, folder=?5, agent=?6 WHERE id=?1",
        params![
            id,
            title,
            pick(&e.notes, cur.notes),
            pick(&e.prompt, cur.prompt),
            pick(&e.folder, cur.folder),
            pick(&e.agent, cur.agent)
        ],
    )?;
    get(db, id)?.context("idea vanished")
}

/// One idea by id.
pub fn get(db: &Db, id: i64) -> Result<Option<Idea>> {
    Ok(db
        .conn
        .query_row("SELECT * FROM idea WHERE id=?1", [id], from_row)
        .optional()?)
}

/// Ideas that are not archived and have no session yet, in board order. A start whose session
/// never showed up (the agent opened, nothing was typed) leaves the idea here.
pub fn list_open(db: &Db) -> Result<Vec<Idea>> {
    let mut st = db.conn.prepare(
        "SELECT * FROM idea WHERE archived=0
           AND (session_id IS NULL OR NOT EXISTS (SELECT 1 FROM session s WHERE s.id=idea.session_id))
         ORDER BY sort, id",
    )?;
    let rows = st.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every idea that is not archived, started or not, in board order.
pub fn list_all(db: &Db) -> Result<Vec<Idea>> {
    let mut st = db
        .conn
        .prepare("SELECT * FROM idea WHERE archived=0 ORDER BY sort, id")?;
    let rows = st.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Record (or clear) the session an idea started.
pub fn set_session(db: &Db, id: i64, sid: Option<&str>) -> Result<()> {
    db.conn.execute(
        "UPDATE idea SET session_id=?2 WHERE id=?1",
        params![id, sid],
    )?;
    Ok(())
}

/// The session the idea started, once eye has seen it.
pub fn started_session(db: &Db, idea: &Idea) -> Result<Option<String>> {
    let Some(sid) = &idea.session_id else {
        return Ok(None);
    };
    let seen = db
        .conn
        .prepare("SELECT 1 FROM session WHERE id=?1")?
        .exists([sid])?;
    Ok(seen.then(|| sid.clone()))
}

/// An agent that cannot be told the id of its session leaves a start unlinked: the scanner links
/// the idea to the first session that begins in its folder, but only within this long.
const LINK_WINDOW_MS: i64 = 60 * 60 * 1000;

/// Record that an idea was started: the session it will have when the agent let eye choose its id,
/// else the time, so the scanner can find the session by its folder once it shows up.
pub fn record_start(db: &Db, id: i64, session_id: Option<&str>) -> Result<()> {
    match session_id {
        Some(_) => set_session(db, id, session_id),
        None => meta_set(&db.conn, &format!("idea_fg:{id}"), &now_ms().to_string()),
    }
}

/// Take back what [`record_start`] recorded: the start did not happen.
pub fn undo_start(db: &Db, id: i64) -> Result<()> {
    set_session(db, id, None)?;
    db.conn
        .execute("DELETE FROM meta WHERE key=?1", [format!("idea_fg:{id}")])?;
    Ok(())
}

/// Take an idea off the board.
pub fn archive(db: &Db, id: i64) -> Result<()> {
    db.conn
        .execute("UPDATE idea SET archived=1 WHERE id=?1", [id])?;
    Ok(())
}

/// The first message of a new session for the idea: its prompt, never its notes.
fn start_prompt(idea: &Idea) -> Option<&str> {
    clean(idea.prompt.as_deref())
}

/// The agent an idea starts in: the one it names, else the default agent. A named agent that is
/// not enabled is an error.
pub fn agent_of(agents: &Agents, idea: &Idea) -> Result<AgentRef> {
    match idea.agent.as_deref().filter(|a| !a.is_empty()) {
        Some(id) => agents
            .get(id)
            .cloned()
            .with_context(|| format!("the agent '{id}' of this idea is not enabled")),
        None => agents
            .default_agent()
            .cloned()
            .context("no agent is enabled (see [agents] in config.toml)"),
    }
}

/// What starting an idea would run.
#[derive(Clone, Debug)]
pub struct IdeaStart {
    pub agent: AgentRef,
    pub dir: String,
    pub invocation: Invocation,
    /// The session the start will create, when the agent lets eye choose its id.
    pub session_id: Option<String>,
}

impl IdeaStart {
    /// The line to paste into a terminal.
    pub fn command(&self) -> String {
        self.invocation.shell_line(&self.dir)
    }
}

/// The command that would start the idea. Foreground picks the session id up front (or reuses the
/// one of a start whose session never showed up) and, without a prompt, opens the agent with
/// nothing sent; a background start needs a prompt, and its session is linked later through its job.
pub fn plan_start(agents: &Agents, idea: &Idea, background: bool) -> Result<IdeaStart> {
    let agent = agent_of(agents, idea)?;
    let dir = idea
        .folder
        .clone()
        .context("this idea has no folder; add one first")?;
    let plan = agent.start(&StartRequest {
        dir: &dir,
        title: &idea.title,
        prompt: start_prompt(idea),
        background,
        session_id: idea.session_id.as_deref().filter(|_| !background),
    })?;
    Ok(IdeaStart {
        agent,
        dir,
        invocation: plan.invocation,
        session_id: plan.session_id,
    })
}

/// What `start` did (or, with `dry_run`, would do).
#[derive(Debug, Clone)]
pub struct Started {
    pub dry_run: bool,
    pub background: bool,
    pub command: String,
    pub applescript: Option<String>,
    pub mode: resume::Mode,
    /// The Linux terminal that opens the session, when it is known.
    pub terminal: Option<String>,
    pub session_id: Option<String>,
    /// Background start: the short job id the agent printed.
    pub job: Option<String>,
    /// What the user should be told: what the agent printed for a background start, what became
    /// of the command (a tab opened, copied) for a foreground one.
    pub output: String,
    /// The folder was missing and the start made it (a dry run: would make it).
    pub creates_folder: bool,
}

/// How to start an idea.
#[derive(Debug, Clone, Copy, Default)]
pub struct StartOpts {
    pub background: bool,
    /// Report what would run; start nothing and create nothing.
    pub dry_run: bool,
    /// Background start: wait a few seconds for the session to show up.
    pub wait_for_job: bool,
    /// Make the idea's folder when it does not exist.
    pub create_folder: bool,
}

/// Check that the idea's folder is there, or may be made (and `create` says to). Returns whether
/// it needs making.
pub fn check_folder(cfg: &Config, dir: &str, create: bool) -> Result<bool> {
    match folder::classify(cfg.folder_root().as_deref(), dir) {
        FolderState::Exists => Ok(false),
        FolderState::Missing { creatable: true } if create => Ok(true),
        FolderState::Missing { creatable: true } => Err(FolderMissing(dir.to_string()).into()),
        other => anyhow::bail!("{}", other.note().unwrap_or_default()),
    }
}

/// Start an idea. Foreground: pick the session id, store it on the idea, open an iTerm tab or a
/// terminal window (or copy the command). Background: run the agent's background command and link
/// the idea through its job. A folder that does not exist is made only when `create_folder` is set.
pub fn start(db: &Db, cfg: &Config, paths: &Paths, id: i64, opts: &StartOpts) -> Result<Started> {
    let StartOpts {
        background,
        dry_run,
        wait_for_job,
        create_folder,
    } = *opts;
    let idea = get(db, id)?.context("no such idea")?;
    if let Some(s) = started_session(db, &idea)?.filter(|_| !dry_run) {
        anyhow::bail!("this idea was already started (session {s})");
    }
    let agents = agents::enabled(cfg);
    let plan = plan_start(&agents, &idea, background)?;
    let creates_folder = check_folder(cfg, &plan.dir, create_folder)?;
    let command = plan.command();
    let target = if background {
        resume::Target::copy()
    } else {
        resume::target(cfg, paths)
    };
    let run = resume::plan_command(command.clone(), &target);
    let mut out = Started {
        dry_run,
        background,
        command: command.clone(),
        applescript: run.applescript.clone(),
        mode: run.mode(),
        terminal: target.terminal().map(String::from),
        session_id: plan.session_id.clone(),
        job: None,
        output: String::new(),
        creates_folder,
    };
    if dry_run {
        return Ok(out);
    }
    if creates_folder {
        folder::create(cfg.folder_root().as_deref(), &plan.dir)?;
    }
    if background {
        // A foreground start that never got a session gives way, so the job can link.
        set_session(db, id, None)?;
        let text = resume::run_shell(&command)?;
        out.output = text.trim().to_string();
        let short = plan.agent.background_job_id(&text).with_context(|| {
            format!(
                "{} did not start a background session: {}",
                plan.agent.short_name(),
                text.trim()
            )
        })?;
        crate::db::meta_set(&db.conn, &format!("idea_bg:{id}"), &short)?;
        if wait_for_job {
            for _ in 0..10 {
                link_sessions(&db.conn, &agents)?;
                if get(db, id)?.is_some_and(|i| i.session_id.is_some()) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        out.session_id = get(db, id)?.and_then(|i| i.session_id);
        out.job = Some(short);
    } else {
        record_start(db, id, plan.session_id.as_deref())?;
        match resume::execute(&run) {
            Ok(note) => out.output = note,
            Err(e) => {
                undo_start(db, id)?;
                return Err(e);
            }
        }
    }
    Ok(out)
}

/// Link ideas and sessions: finish background starts through the agents' jobs, then
/// point each session at the idea that started it.
pub fn link_sessions(tx: &Connection, agents: &Agents) -> Result<()> {
    let mut pend: Vec<(String, String)> = vec![];
    {
        let mut st = tx.prepare("SELECT key, value FROM meta WHERE key LIKE 'idea_bg:%'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        for r in rows {
            pend.push(r?);
        }
    }
    if !pend.is_empty() {
        let jobs: Vec<_> = agents.iter().flat_map(|a| a.jobs()).collect();
        for (key, short) in pend {
            let Some(sid) = jobs
                .iter()
                .find(|j| j.short == short)
                .and_then(|j| j.session_id.clone())
            else {
                continue;
            };
            if let Some(n) = key
                .strip_prefix("idea_bg:")
                .and_then(|n| n.parse::<i64>().ok())
            {
                tx.execute(
                    "UPDATE idea SET session_id=?2 WHERE id=?1 AND session_id IS NULL",
                    params![n, sid],
                )?;
            }
            tx.execute("DELETE FROM meta WHERE key=?1", [&key])?;
        }
    }
    link_foreground_starts(tx, agents)?;
    tx.execute(
        "UPDATE session SET idea_id=(SELECT i.id FROM idea i WHERE i.session_id=session.id)
         WHERE idea_id IS NULL AND id IN (SELECT session_id FROM idea WHERE session_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

/// An idea that was started and has no session yet.
struct Unlinked {
    id: i64,
    agent: Option<String>,
    folder: String,
    /// What the start sent as the first message, when it sent one.
    prompt: Option<String>,
}

/// The idea of a pending start (`idea_fg:<n>`), when it still waits for its session: it is there,
/// not archived, has a folder and no session.
fn unlinked(tx: &Connection, key: &str) -> Result<Option<Unlinked>> {
    let Some(id) = key
        .strip_prefix("idea_fg:")
        .and_then(|n| n.parse::<i64>().ok())
    else {
        return Ok(None);
    };
    Ok(tx
        .query_row(
            "SELECT agent, folder, prompt FROM idea WHERE id=?1 AND archived=0 AND session_id IS NULL",
            [id],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
        .and_then(|(agent, folder, prompt)| {
            Some(Unlinked {
                id,
                agent,
                folder: folder?,
                prompt: clean(prompt.as_deref()).map(String::from),
            })
        }))
}

/// How much of a prompt is compared with the first message of a session.
const PROMPT_MATCH_CHARS: usize = 200;

/// Is `first` the first message an idea with this prompt sends? A start puts `Task: ` before a
/// prompt that could be read as a flag or a command, so that is ignored.
fn starts_with_prompt(first: &str, prompt: &str) -> bool {
    let norm = |t: &str| -> String {
        let t = t.trim();
        t.strip_prefix("Task: ")
            .unwrap_or(t)
            .chars()
            .take(PROMPT_MATCH_CHARS)
            .collect()
    };
    norm(first) == norm(prompt)
}

/// The first session of the idea's agent that began in its folder at or after `since` and belongs
/// to no idea. When the idea sent a prompt, the session's first message must be it, so a second
/// terminal in the same folder does not take the link.
fn first_session_since(
    tx: &Connection,
    agents: &Agents,
    idea: &Unlinked,
    since: i64,
) -> Result<Option<String>> {
    let Some(agent) = agents.resolve(idea.agent.as_deref()) else {
        return Ok(None);
    };
    let mut st = tx.prepare(
        "SELECT id, first_prompt FROM session
         WHERE agent=?1 AND rtrim(cwd,'/')=rtrim(?2,'/') AND started_at>=?3 AND idea_id IS NULL
           AND id NOT IN (SELECT session_id FROM idea WHERE session_id IS NOT NULL)
         ORDER BY started_at, id",
    )?;
    let rows = st.query_map(params![agent.id(), idea.folder, since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (id, first) = row?;
        match (&idea.prompt, first) {
            (None, _) => return Ok(Some(id)),
            (Some(p), Some(f)) if starts_with_prompt(&f, p) => return Ok(Some(id)),
            _ => {}
        }
    }
    Ok(None)
}

/// Finish the starts that left the session id to the agent: the idea takes the first session of its
/// agent that began in its folder since the start. A start nobody typed into (no session by then)
/// is let go after [`LINK_WINDOW_MS`].
fn link_foreground_starts(tx: &Connection, agents: &Agents) -> Result<()> {
    let mut pend: Vec<(String, String)> = vec![];
    {
        let mut st = tx.prepare("SELECT key, value FROM meta WHERE key LIKE 'idea_fg:%'")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        for r in rows {
            pend.push(r?);
        }
    }
    for (key, started) in pend {
        let started: i64 = started.parse().unwrap_or(0);
        let idea = unlinked(tx, &key)?;
        let found = match &idea {
            Some(i) => first_session_since(tx, agents, i, started)?,
            None => None,
        };
        if let (Some(i), Some(sid)) = (&idea, &found) {
            tx.execute(
                "UPDATE idea SET session_id=?2 WHERE id=?1 AND session_id IS NULL",
                params![i.id, sid],
            )?;
        }
        if found.is_some() || idea.is_none() || now_ms() - started > LINK_WINDOW_MS {
            tx.execute("DELETE FROM meta WHERE key=?1", [&key])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod folder_tests;

#[cfg(test)]
mod tests;
