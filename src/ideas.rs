use crate::agents::{self, AgentRef, Agents, Invocation, StartRequest};
use crate::config::Config;
use crate::db::{meta_set, Db};
use crate::model::Idea;
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
    pub session_id: Option<String>,
    /// Background start: the short job id the agent printed.
    pub job: Option<String>,
    /// What the user should be told: what the agent printed for a background start, what became
    /// of the command (a tab opened, copied) for a foreground one.
    pub output: String,
}

/// Start an idea. Foreground: pick the session id, store it on the idea, open an iTerm tab (or
/// copy the command). Background: run the agent's background command and link the idea through
/// its job.
pub fn start(
    db: &Db,
    cfg: &Config,
    id: i64,
    background: bool,
    dry_run: bool,
    wait_for_job: bool,
) -> Result<Started> {
    let idea = get(db, id)?.context("no such idea")?;
    if let Some(s) = started_session(db, &idea)?.filter(|_| !dry_run) {
        anyhow::bail!("this idea was already started (session {s})");
    }
    let agents = agents::enabled(cfg);
    let plan = plan_start(&agents, &idea, background)?;
    let command = plan.command();
    let mode = if background {
        resume::Mode::Copy
    } else {
        resume::Mode::from_config(cfg)
    };
    let run = resume::plan_command(command.clone(), mode.clone());
    let mut out = Started {
        dry_run,
        background,
        command: command.clone(),
        applescript: run.applescript.clone(),
        mode,
        session_id: plan.session_id.clone(),
        job: None,
        output: String::new(),
    };
    if dry_run {
        return Ok(out);
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
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use std::sync::Arc;

    fn claude_only() -> Agents {
        Agents::from_vec(vec![Arc::new(Claude::new("/nonexistent/c".into()))])
    }

    fn new<'a>(title: &'a str, folder: Option<&'a str>) -> NewIdea<'a> {
        NewIdea {
            title,
            folder,
            ..NewIdea::default()
        }
    }

    #[test]
    fn add_list_plan() {
        let db = Db::open_memory().unwrap();
        let agents = claude_only();
        assert!(add(&db, &new("  ", None)).is_err());
        let i = add(
            &db,
            &NewIdea {
                notes: Some("notes here"),
                prompt: Some("do it"),
                ..new("Try it", Some("/tmp/x y"))
            },
        )
        .unwrap();
        assert_eq!(list_open(&db).unwrap().len(), 1);
        let p = plan_start(&agents, &i, false).unwrap();
        let id = p.session_id.clone().unwrap();
        assert_eq!(id.len(), 36);
        assert!(p
            .command()
            .starts_with("cd '/tmp/x y' && claude --session-id "));
        // the prompt is sent, the notes never are
        assert!(p.command().ends_with("-n 'Try it' 'do it'"));
        assert!(!p.command().contains("notes here"));
        let b = plan_start(&agents, &i, true).unwrap();
        assert!(b.session_id.is_none() && b.command().contains("--bg"));
        // without a prompt the agent opens with nothing sent; the background needs one
        let quiet = add(
            &db,
            &NewIdea {
                notes: Some("just notes"),
                ..new("Quiet", Some("/tmp/x"))
            },
        )
        .unwrap();
        assert!(plan_start(&agents, &quiet, false)
            .unwrap()
            .command()
            .ends_with("-n Quiet"));
        assert!(plan_start(&agents, &quiet, true).is_err());
        let j = add(&db, &new("No folder", None)).unwrap();
        assert!(plan_start(&agents, &j, false).is_err());
        archive(&db, i.id).unwrap();
        assert_eq!(list_open(&db).unwrap().len(), 2);
    }

    #[test]
    fn an_idea_starts_in_the_agent_it_names() {
        let db = Db::open_memory().unwrap();
        let agents = claude_only();
        let i = add(&db, &new("Try it", Some("/tmp/x"))).unwrap();
        assert_eq!(i.agent, None);
        assert_eq!(agent_of(&agents, &i).unwrap().id(), "claude");
        let named = add(
            &db,
            &NewIdea {
                agent: Some("claude"),
                ..new("Named", Some("/tmp/x"))
            },
        )
        .unwrap();
        assert_eq!(named.agent.as_deref(), Some("claude"));
        let off = add(
            &db,
            &NewIdea {
                agent: Some("elsewhere"),
                ..new("Off", Some("/tmp/x"))
            },
        )
        .unwrap();
        assert!(plan_start(&agents, &off, false)
            .unwrap_err()
            .to_string()
            .contains("not enabled"));
        // no agent enabled: nothing to start in
        assert!(plan_start(&Agents::default(), &i, false).is_err());
    }

    #[test]
    fn start_dry_run_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = Config::with_claude_dir(&tmp.path().join("c"));
        let db = Db::open_memory().unwrap();
        let i = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Try it", Some("/tmp/x"))
            },
        )
        .unwrap();
        let st = start(&db, &cfg, i.id, false, true, false).unwrap();
        assert!(st.dry_run && st.command.contains("--session-id"));
        assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
        let bg = start(&db, &cfg, i.id, true, true, false).unwrap();
        assert!(bg.command.contains("claude --bg") && bg.session_id.is_none());
    }

    #[test]
    fn background_job_takes_over_the_idea() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dir = tmp.path().join("c");
        std::fs::create_dir_all(claude_dir.join("jobs/30194328")).unwrap();
        std::fs::write(
            claude_dir.join("jobs/30194328/state.json"),
            r#"{"state":"done","sessionId":"30194328-e28b-49bc-8d12-e13b2b0670db"}"#,
        )
        .unwrap();
        let agents = Agents::from_vec(vec![Arc::new(Claude::new(claude_dir))]);
        let db = Db::open_memory().unwrap();
        let i = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Try it", Some("/tmp/x"))
            },
        )
        .unwrap();
        crate::db::meta_set(&db.conn, &format!("idea_bg:{}", i.id), "30194328").unwrap();
        db.conn
            .execute(
                "INSERT INTO session(id) VALUES('30194328-e28b-49bc-8d12-e13b2b0670db')",
                [],
            )
            .unwrap();
        link_sessions(&db.conn, &agents).unwrap();
        let i = get(&db, i.id).unwrap().unwrap();
        assert_eq!(
            i.session_id.as_deref(),
            Some("30194328-e28b-49bc-8d12-e13b2b0670db")
        );
        assert!(list_open(&db).unwrap().is_empty());
        let linked: Option<i64> = db
            .conn
            .query_row("SELECT idea_id FROM session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(linked, Some(i.id));
    }

    /// A session row as the scanner leaves it: its agent, folder and start time.
    fn session(db: &Db, id: &str, agent: &str, cwd: &str, started: i64) {
        session_with(db, id, agent, cwd, started, "go");
    }

    fn session_with(db: &Db, id: &str, agent: &str, cwd: &str, started: i64, first: &str) {
        db.conn
            .execute(
                "INSERT INTO session(id, agent, cwd, started_at, first_prompt) VALUES(?1,?2,?3,?4,?5)",
                params![id, agent, cwd, started, first],
            )
            .unwrap();
    }

    fn codex_only() -> Agents {
        Agents::from_vec(vec![Arc::new(crate::agents::codex::Codex::new(
            "/nonexistent/codex".into(),
        ))])
    }

    fn pending(db: &Db, id: i64) -> bool {
        db.meta_get(&format!("idea_fg:{id}")).unwrap().is_some()
    }

    #[test]
    fn a_start_the_agent_names_no_session_for_links_to_the_next_session_in_the_folder() {
        let db = Db::open_memory().unwrap();
        let agents = codex_only();
        let i = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Try it", Some("/work/p/"))
            },
        )
        .unwrap();
        let plan = plan_start(&agents, &i, false).unwrap();
        assert!(plan.session_id.is_none());
        record_start(&db, i.id, plan.session_id.as_deref()).unwrap();
        assert!(pending(&db, i.id) && get(&db, i.id).unwrap().unwrap().session_id.is_none());
        let t0: i64 = db
            .meta_get(&format!("idea_fg:{}", i.id))
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        // sessions that do not belong to it: older, in another folder, of another agent
        session(&db, "old-session", "codex", "/work/p", t0 - 5_000);
        session(&db, "elsewhere", "codex", "/work/other", t0 + 1_000);
        session(&db, "claude-one", "claude", "/work/p", t0 + 1_000);
        link_sessions(&db.conn, &agents).unwrap();
        assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
        assert!(pending(&db, i.id), "still waiting for its session");
        // the session it started, then a later one in the same folder
        session(&db, "the-one", "codex", "/work/p", t0 + 2_000);
        session(&db, "a-later-one", "codex", "/work/p", t0 + 90_000);
        link_sessions(&db.conn, &agents).unwrap();
        assert_eq!(
            get(&db, i.id).unwrap().unwrap().session_id.as_deref(),
            Some("the-one")
        );
        assert!(!pending(&db, i.id));
        let linked: Option<i64> = db
            .conn
            .query_row("SELECT idea_id FROM session WHERE id='the-one'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(linked, Some(i.id));
        // a second idea in the same folder does not take the session of the first
        let j = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Again", Some("/work/p"))
            },
        )
        .unwrap();
        record_start(&db, j.id, None).unwrap();
        db.conn
            .execute(
                "UPDATE meta SET value=?2 WHERE key=?1",
                params![format!("idea_fg:{}", j.id), t0.to_string()],
            )
            .unwrap();
        link_sessions(&db.conn, &agents).unwrap();
        assert_eq!(
            get(&db, j.id).unwrap().unwrap().session_id.as_deref(),
            Some("a-later-one")
        );
    }

    #[test]
    fn a_session_of_another_terminal_in_the_folder_does_not_take_the_link() {
        let db = Db::open_memory().unwrap();
        let agents = codex_only();
        let i = add(
            &db,
            &NewIdea {
                prompt: Some("Try a jittered backoff"),
                ..new("Jitter", Some("/work/p"))
            },
        )
        .unwrap();
        record_start(&db, i.id, None).unwrap();
        let t0: i64 = db
            .meta_get(&format!("idea_fg:{}", i.id))
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        session_with(
            &db,
            "somebody-else",
            "codex",
            "/work/p",
            t0 + 1_000,
            "Fix the build",
        );
        link_sessions(&db.conn, &agents).unwrap();
        assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
        assert!(pending(&db, i.id));
        session_with(
            &db,
            "the-one",
            "codex",
            "/work/p",
            t0 + 5_000,
            "Try a jittered backoff",
        );
        link_sessions(&db.conn, &agents).unwrap();
        assert_eq!(
            get(&db, i.id).unwrap().unwrap().session_id.as_deref(),
            Some("the-one")
        );
        // a prompt eye prefixed with `Task: ` (one word, or one that starts with a dash) still matches
        assert!(starts_with_prompt("Task: review", "review"));
        assert!(starts_with_prompt("  Try it  ", "Try it"));
        assert!(!starts_with_prompt("Try it again", "Try it"));
        let long = "x".repeat(500);
        assert!(
            starts_with_prompt(&long, &long),
            "a long prompt is compared by its start"
        );
    }

    #[test]
    fn a_start_that_never_got_a_session_is_let_go_or_taken_back() {
        let db = Db::open_memory().unwrap();
        let agents = codex_only();
        let i = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Stale", Some("/work/p"))
            },
        )
        .unwrap();
        record_start(&db, i.id, None).unwrap();
        // an hour later nobody typed into it
        let long_ago = (now_ms() - LINK_WINDOW_MS - 1_000).to_string();
        db.conn
            .execute(
                "UPDATE meta SET value=?2 WHERE key=?1",
                params![format!("idea_fg:{}", i.id), long_ago],
            )
            .unwrap();
        link_sessions(&db.conn, &agents).unwrap();
        assert!(!pending(&db, i.id));
        assert!(get(&db, i.id).unwrap().unwrap().session_id.is_none());
        // a launch that failed is taken back
        record_start(&db, i.id, None).unwrap();
        undo_start(&db, i.id).unwrap();
        assert!(!pending(&db, i.id));
        // an idea that was archived or deleted leaves nothing behind
        record_start(&db, i.id, None).unwrap();
        archive(&db, i.id).unwrap();
        link_sessions(&db.conn, &agents).unwrap();
        assert!(!pending(&db, i.id));
        // an agent that lets eye choose the id never waits
        let c = add(
            &db,
            &NewIdea {
                prompt: Some("go"),
                ..new("Claude", Some("/work/p"))
            },
        )
        .unwrap();
        let claude = claude_only();
        let plan = plan_start(&claude, &c, false).unwrap();
        record_start(&db, c.id, plan.session_id.as_deref()).unwrap();
        assert!(!pending(&db, c.id));
        assert_eq!(get(&db, c.id).unwrap().unwrap().session_id, plan.session_id);
    }

    #[test]
    fn edit_changes_only_what_is_given() {
        let db = Db::open_memory().unwrap();
        let i = add(
            &db,
            &NewIdea {
                notes: Some("mine"),
                ..new("Try it", Some("/tmp/x"))
            },
        )
        .unwrap();
        let e = |f: fn(&mut Edit)| {
            let mut e = Edit::default();
            f(&mut e);
            e
        };
        let i2 = edit(&db, i.id, &e(|e| e.prompt = Some(" do it ".into()))).unwrap();
        assert_eq!(
            (
                i2.title.as_str(),
                i2.notes.as_deref(),
                i2.prompt.as_deref(),
                i2.folder.as_deref()
            ),
            ("Try it", Some("mine"), Some("do it"), Some("/tmp/x"))
        );
        let i3 = edit(&db, i.id, &e(|e| e.notes = Some("  ".into()))).unwrap();
        assert_eq!((i3.notes, i3.prompt.as_deref()), (None, Some("do it")));
        let i4 = edit(&db, i.id, &e(|e| e.agent = Some("claude".into()))).unwrap();
        assert_eq!(i4.agent.as_deref(), Some("claude"));
        let i5 = edit(&db, i.id, &e(|e| e.agent = Some(String::new()))).unwrap();
        assert_eq!(i5.agent, None);
        assert!(edit(&db, i.id, &e(|e| e.title = Some(" ".into()))).is_err());
        assert!(edit(&db, 99, &Edit::default()).is_err());
    }

    #[test]
    fn a_start_whose_session_never_came_stays_an_idea() {
        let db = Db::open_memory().unwrap();
        let i = add(&db, &new("Try it", Some("/tmp/x"))).unwrap();
        set_session(&db, i.id, Some("sid-1")).unwrap();
        let i = get(&db, i.id).unwrap().unwrap();
        assert_eq!(list_open(&db).unwrap().len(), 1);
        assert_eq!(started_session(&db, &i).unwrap(), None);
        // starting again reuses the id, so a late first message still lands on the idea
        assert_eq!(
            plan_start(&claude_only(), &i, false)
                .unwrap()
                .session_id
                .as_deref(),
            Some("sid-1")
        );
        db.conn
            .execute("INSERT INTO session(id) VALUES('sid-1')", [])
            .unwrap();
        assert!(list_open(&db).unwrap().is_empty());
        assert_eq!(started_session(&db, &i).unwrap().as_deref(), Some("sid-1"));
    }

    #[test]
    fn start_plan_survives_nasty_strings() {
        let nasty = "it's \"q\" \\ $(touch /tmp/pwn) `id`\nline2";
        let mut idea = Idea {
            id: 1,
            title: nasty.into(),
            prompt: Some("--dangerously-skip-permissions $(id)".into()),
            folder: Some(format!("/{nasty}")),
            session_id: None,
            ..Default::default()
        };
        let agents = claude_only();
        let p = plan_start(&agents, &idea, true).unwrap();
        assert!(p
            .command()
            .contains("'Task: --dangerously-skip-permissions $(id)'"));
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(
                p.command()
                    .replace("cd ", "printf '%s\\n' ")
                    .replace("&& claude", "; printf '%s\\n' claude"),
            )
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).starts_with(&format!("/{nasty}")));
        idea.prompt = None;
        assert!(plan_start(&agents, &idea, false)
            .unwrap()
            .command()
            .contains("--session-id "));
    }
}
