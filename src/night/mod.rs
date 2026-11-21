//! The night shift: planned runs that start by themselves inside a time window while you are
//! away, each in its own git worktree (or a copy of a plain folder), never pushing anything.
//!
//! * `clock`: the window, which night a moment belongs to, local dates.
//! * this file: the `night_run` table, limits and planning.
//! * `iso`: the worktree or copy a run works in, its diff, and discarding it.
//! * `barrier`: the sandbox and deny rules that keep a Claude run out of the user's own folders.
//! * `launch`: the command line of a run for each agent and starting it detached.
//! * `sched`: the once-a-minute check that starts, watches, stops and finishes runs.
//!
//! Nothing runs unless `[night] enabled = true` and a run is planned. See `docs/night.md`.

pub mod barrier;
pub mod clock;
pub mod iso;
pub mod launch;
pub mod sched;
#[cfg(test)]
mod tests;

use crate::agents::Agents;
use crate::config::Config;
use crate::db::Db;
use crate::paths::Paths;
use crate::util::now_ms;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const PLANNED: &str = "planned";
/// A run a starter has claimed and is launching: nobody else starts or cancels it.
pub const STARTING: &str = "starting";
pub const RUNNING: &str = "running";
/// A run eye started but cannot follow or signal (no process start time to tell it from a reused
/// pid). It is left alone; its folder is still there.
pub const LOST: &str = "lost";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const STOPPED: &str = "stopped";
pub const SKIPPED: &str = "skipped";

/// The next step a finished run puts on its session's card.
pub const NEXT_STEP: &str = "review the night run";

/// The one optional limit of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// Stop when the agent's weekly plan is above this many percent.
    WeeklyPercent(i64),
    /// Stop when the run's session has used this many tokens (input, output and cache writes of
    /// the run and its sub-agents; cache reads are not counted).
    Tokens(i64),
    /// Stop after this many minutes.
    Minutes(i64),
}

impl Limit {
    /// `30%`, `2M` or `500k` (tokens: capital M, either k), `90m`, `2h`.
    pub fn parse(s: &str) -> Result<Limit> {
        let t = s.trim();
        let bad = || {
            anyhow::anyhow!(
                "'{s}' is not a limit: write 30% (weekly plan), 2M or 500k (tokens), 90m or 2h (time)"
            )
        };
        let num = |x: &str| x.trim().parse::<f64>().ok().filter(|n| *n > 0.0);
        if let Some(n) = t.strip_suffix('%') {
            let n = num(n).ok_or_else(bad)?;
            anyhow::ensure!(n <= 100.0, "a weekly limit is a percent from 1 to 100");
            return Ok(Limit::WeeklyPercent(n.round() as i64));
        }
        if let Some(n) = t.strip_suffix('M') {
            return Ok(Limit::Tokens((num(n).ok_or_else(bad)? * 1e6) as i64));
        }
        if let Some(n) = t.strip_suffix('k').or_else(|| t.strip_suffix('K')) {
            return Ok(Limit::Tokens((num(n).ok_or_else(bad)? * 1e3) as i64));
        }
        if let Some(n) = t.strip_suffix('h') {
            return Ok(Limit::Minutes((num(n).ok_or_else(bad)? * 60.0) as i64));
        }
        if let Some(n) = t.strip_suffix("min").or_else(|| t.strip_suffix('m')) {
            return Ok(Limit::Minutes(num(n).ok_or_else(bad)? as i64));
        }
        Err(bad())
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Limit::WeeklyPercent(_) => "weekly",
            Limit::Tokens(_) => "tokens",
            Limit::Minutes(_) => "minutes",
        }
    }

    pub fn value(&self) -> i64 {
        match *self {
            Limit::WeeklyPercent(v) | Limit::Tokens(v) | Limit::Minutes(v) => v,
        }
    }

    pub fn from_parts(kind: Option<&str>, value: Option<i64>) -> Option<Limit> {
        let v = value?;
        match kind? {
            "weekly" => Some(Limit::WeeklyPercent(v)),
            "tokens" => Some(Limit::Tokens(v)),
            "minutes" => Some(Limit::Minutes(v)),
            _ => None,
        }
    }

    /// In words: `weekly plan above 30%`, `2.0M tokens`, `90 minutes`.
    pub fn text(&self) -> String {
        match *self {
            Limit::WeeklyPercent(v) => format!("weekly plan above {v}%"),
            Limit::Tokens(v) => format!("{} tokens", crate::util::fmt_tokens(v)),
            Limit::Minutes(v) => format!("{v} minutes"),
        }
    }
}

/// One planned or finished run.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub id: i64,
    /// `idea` or `session`.
    pub kind: String,
    pub idea_id: Option<i64>,
    /// The session a `session` run continues.
    pub session_id: Option<String>,
    pub agent: String,
    pub switch_ok: bool,
    pub folder: String,
    pub prompt: String,
    /// The local date of the night it is planned for.
    pub night: String,
    pub status: String,
    pub why: Option<String>,
    pub limit_kind: Option<String>,
    pub limit_value: Option<i64>,
    /// The session the run created.
    pub run_session_id: Option<String>,
    pub pid: Option<i64>,
    pub pid_start: Option<i64>,
    pub worktree: Option<String>,
    pub branch: Option<String>,
    pub base_ref: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub report: Option<String>,
    pub diff_stat: Option<String>,
    pub created_at: i64,
    /// `worktree` or `copy`; None (a run planned before the column) is a worktree.
    pub isolation: Option<String>,
}

/// How a run keeps the user's folder safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A git worktree of the repo on a `night/...` branch.
    Worktree,
    /// A separate local copy: the original folder can be locked completely.
    Copy,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode> {
        match s.trim() {
            "worktree" => Ok(Mode::Worktree),
            "copy" => Ok(Mode::Copy),
            o => bail!("'{o}' is not an isolation: write worktree or copy"),
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Mode::Worktree => "worktree",
            Mode::Copy => "copy",
        }
    }
}

impl Run {
    pub fn mode(&self) -> Mode {
        self.isolation
            .as_deref()
            .and_then(|s| Mode::parse(s).ok())
            .unwrap_or(Mode::Worktree)
    }

    /// A copy of a git repo (has a night branch): the one BRING INTO MY REPO is for.
    pub fn is_repo_copy(&self) -> bool {
        self.mode() == Mode::Copy && self.branch.is_some() && self.worktree.is_some()
    }

    pub fn limit(&self) -> Option<Limit> {
        Limit::from_parts(self.limit_kind.as_deref(), self.limit_value)
    }

    /// A run that has not ended: planned, or still going.
    pub fn is_open(&self) -> bool {
        self.status == PLANNED || self.status == STARTING || self.status == RUNNING
    }

    /// The folder the run works in (the worktree, or the copy).
    pub fn work_dir(&self) -> Option<PathBuf> {
        self.worktree.as_deref().map(PathBuf::from)
    }

    pub fn to_json(&self, now: i64) -> Value {
        let end = self.ended_at.unwrap_or(now);
        json!({
            "id": self.id, "kind": self.kind, "idea_id": self.idea_id,
            "session_id": self.session_id, "agent": self.agent, "switch": self.switch_ok,
            "folder": self.folder, "prompt": self.prompt, "night": self.night,
            "status": self.status, "why": self.why,
            "limit": self.limit().map(|l| json!({"kind": l.kind(), "value": l.value(), "text": l.text()})),
            "run_session_id": self.run_session_id, "worktree": self.worktree,
            "branch": self.branch, "base_ref": self.base_ref,
            "started_at": self.started_at, "ended_at": self.ended_at,
            "duration_ms": self.started_at.map(|s| (end - s).max(0)),
            "report": self.report, "diff_stat": self.diff_stat,
            "created_at": self.created_at,
            "isolation": self.mode().text(),
        })
    }
}

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: r.get("id")?,
        kind: r.get("kind")?,
        idea_id: r.get("idea_id")?,
        session_id: r.get("session_id")?,
        agent: r.get("agent")?,
        switch_ok: r.get::<_, i64>("switch_ok")? != 0,
        folder: r.get("folder")?,
        prompt: r.get("prompt")?,
        night: r.get("night")?,
        status: r.get("status")?,
        why: r.get("why")?,
        limit_kind: r.get("limit_kind")?,
        limit_value: r.get("limit_value")?,
        run_session_id: r.get("run_session_id")?,
        pid: r.get("pid")?,
        pid_start: r.get("pid_start")?,
        worktree: r.get("worktree")?,
        branch: r.get("branch")?,
        base_ref: r.get("base_ref")?,
        started_at: r.get("started_at")?,
        ended_at: r.get("ended_at")?,
        report: r.get("report")?,
        diff_stat: r.get("diff_stat")?,
        created_at: r.get("created_at")?,
        isolation: r.get("isolation")?,
    })
}

pub fn get(db: &Db, id: i64) -> Result<Option<Run>> {
    Ok(db
        .conn
        .query_row("SELECT * FROM night_run WHERE id=?1", [id], from_row)
        .optional()?)
}

/// The run that made this session.
pub fn of_session(db: &Db, session_id: &str) -> Result<Option<Run>> {
    Ok(db
        .conn
        .query_row(
            "SELECT * FROM night_run WHERE run_session_id=?1 ORDER BY id DESC LIMIT 1",
            [session_id],
            from_row,
        )
        .optional()?)
}

/// Every run, newest first; `open_only` leaves out the ones that ended.
pub fn list(db: &Db, open_only: bool) -> Result<Vec<Run>> {
    let sql = if open_only {
        "SELECT * FROM night_run WHERE status IN ('planned','running') ORDER BY id"
    } else {
        "SELECT * FROM night_run ORDER BY id DESC"
    };
    let mut st = db.conn.prepare(sql)?;
    let rows = st.query_map([], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Runs in one status, oldest first.
pub fn with_status(db: &Db, status: &str) -> Result<Vec<Run>> {
    let mut st = db
        .conn
        .prepare("SELECT * FROM night_run WHERE status=?1 ORDER BY id")?;
    let rows = st.query_map([status], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// How many runs are planned (the header chip).
pub fn planned_count(db: &Db) -> Result<i64> {
    Ok(db.conn.query_row(
        "SELECT COUNT(*) FROM night_run WHERE status='planned'",
        [],
        |r| r.get(0),
    )?)
}

/// Where a run keeps its log and, unless it is a worktree elsewhere, its folder.
pub fn run_dir(paths: &Paths, id: i64) -> PathBuf {
    paths.home.join("night").join(id.to_string())
}

/// What a plan asks for.
#[derive(Debug, Clone, Default)]
pub struct PlanOpts {
    pub agent: Option<String>,
    pub limit: Option<Limit>,
    pub switch: bool,
    /// For a continued session: what it should do tonight.
    pub prompt: Option<String>,
    /// `worktree` or `copy`; None takes `[night] isolation`.
    pub isolation: Option<String>,
}

/// The mode a plan asks for: the per-run choice, else the config. A plain folder is always a copy.
fn pick_mode(cfg: &Config, o: &PlanOpts, folder: &str) -> Result<Mode> {
    let want = match clean(o.isolation.as_deref()) {
        Some(m) => Mode::parse(m)?,
        None => Mode::parse(&cfg.night.isolation).context("[night] isolation in config.toml")?,
    };
    if iso::repo_root(&crate::loose::System::new(), std::path::Path::new(folder)).is_none() {
        return Ok(Mode::Copy);
    }
    Ok(want)
}

/// An agent that cannot be confined runs only when the config says so, and the plan says why not.
fn check_runnable(cfg: &Config, agent: &crate::agents::AgentRef) -> Result<()> {
    if agent.id() == "opencode" {
        barrier::gate_unsandboxed(cfg)?;
    }
    Ok(())
}

fn clean(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|v| !v.is_empty())
}

/// Can this limit be checked with what eye has? A limit it cannot check is refused, with why.
pub fn check_limit(db: &Db, agent: &crate::agents::AgentRef, l: Limit) -> Result<()> {
    if let Limit::WeeklyPercent(_) = l {
        let have = crate::usage::limits::load(&db.conn, agent.id())?
            .is_some_and(|p| p.windows.iter().any(is_weekly));
        if !have {
            let how = if agent.id() == crate::agents::LEGACY_ID {
                " Run `eye statusline` setup (`eye install`) so Claude Code reports its limits, then plan again."
            } else if agent.id() == "opencode" {
                " OpenCode writes no plan limits to disk (they are the provider's), so use a token limit instead."
            } else {
                " Open a Codex session once so its rollout reports the limits, then plan again."
            };
            bail!(
                "eye has no weekly plan data for {}, so a weekly limit cannot be checked.{how}",
                agent.name()
            );
        }
    }
    Ok(())
}

/// Is this the weekly window of a plan?
pub fn is_weekly(w: &crate::usage::LimitWindow) -> bool {
    crate::usage::limits::label(w) == "weekly"
}

fn insert(db: &Db, r: &Run) -> Result<Run> {
    db.conn.execute(
        "INSERT INTO night_run(kind, idea_id, session_id, agent, switch_ok, folder, prompt, night, status,
                               limit_kind, limit_value, created_at, isolation)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'planned',?9,?10,?11,?12)",
        params![
            r.kind,
            r.idea_id,
            r.session_id,
            r.agent,
            r.switch_ok,
            r.folder,
            r.prompt,
            r.night,
            r.limit_kind,
            r.limit_value,
            r.created_at,
            r.isolation
        ],
    )?;
    get(db, db.conn.last_insert_rowid())?.context("run vanished")
}

fn tonight(cfg: &Config, now: i64) -> Result<String> {
    let w = clock::Window::parse(&cfg.night.window).map_err(anyhow::Error::msg)?;
    Ok(clock::night_of(&w, now, &clock::local_offset))
}

/// Plan an idea for tonight. It needs a folder and a prompt; both are asked for in words.
pub fn plan_idea(
    db: &Db,
    cfg: &Config,
    agents: &Agents,
    idea_id: i64,
    o: &PlanOpts,
) -> Result<Run> {
    let idea = crate::ideas::get(db, idea_id)?.context("no such idea")?;
    let Some(folder) = clean(idea.folder.as_deref()) else {
        bail!("this idea has no folder: add the folder the run should work on, then plan it");
    };
    let Some(prompt) = clean(idea.prompt.as_deref()) else {
        bail!("this idea has no prompt: write what the agent should do (the idea's prompt), then plan it");
    };
    anyhow::ensure!(
        std::path::Path::new(folder).is_dir(),
        "the folder {folder} does not exist"
    );
    let agent = match clean(o.agent.as_deref()) {
        Some(id) => agents
            .get(id)
            .cloned()
            .with_context(|| format!("the agent '{id}' is not enabled"))?,
        None => crate::ideas::agent_of(agents, &idea)?,
    };
    iso::check_plain_size(std::path::Path::new(folder))?;
    check_runnable(cfg, &agent)?;
    let mode = pick_mode(cfg, o, folder)?;
    if let Some(l) = o.limit {
        check_limit(db, &agent, l)?;
    }
    if o.switch {
        anyhow::ensure!(
            agents.iter().count() > 1,
            "switching needs a second enabled agent (see [agents] in config.toml)"
        );
    }
    insert(
        db,
        &Run {
            kind: "idea".into(),
            idea_id: Some(idea.id),
            agent: agent.id().into(),
            switch_ok: o.switch,
            folder: folder.into(),
            prompt: prompt.into(),
            night: tonight(cfg, now_ms())?,
            limit_kind: o.limit.map(|l| l.kind().into()),
            limit_value: o.limit.map(|l| l.value()),
            created_at: now_ms(),
            isolation: Some(mode.text().into()),
            ..Run::default()
        },
    )
}

/// Plan "continue tonight" for a session: a prompt says what it should do.
pub fn plan_session(
    db: &Db,
    cfg: &Config,
    agents: &Agents,
    session: &str,
    o: &PlanOpts,
) -> Result<Run> {
    let id = db.resolve_id(session)?;
    let s = db.session_by_id(&id)?.context("no such session")?;
    let Some(prompt) = clean(o.prompt.as_deref()) else {
        bail!("continue tonight needs a prompt: say what the session should do tonight");
    };
    anyhow::ensure!(
        !o.switch,
        "switching agents is for ideas; a continued session stays on its own agent"
    );
    anyhow::ensure!(
        s.kind.as_deref() != Some("history_only"),
        "this session has no transcript any more; it cannot be continued"
    );
    let (agent, cwd) = agents.runnable(&s).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        agent.fork(&s.id).is_some(),
        "{} cannot fork a session, so it cannot be continued tonight",
        agent.name()
    );
    anyhow::ensure!(
        std::path::Path::new(cwd).is_dir(),
        "the folder {cwd} no longer exists"
    );
    iso::check_plain_size(std::path::Path::new(cwd))?;
    check_runnable(cfg, agent)?;
    let mode = pick_mode(cfg, o, cwd)?;
    if let Some(l) = o.limit {
        check_limit(db, agent, l)?;
    }
    insert(
        db,
        &Run {
            kind: "session".into(),
            session_id: Some(s.id.clone()),
            agent: agent.id().into(),
            folder: cwd.into(),
            prompt: prompt.into(),
            night: tonight(cfg, now_ms())?,
            limit_kind: o.limit.map(|l| l.kind().into()),
            limit_value: o.limit.map(|l| l.value()),
            created_at: now_ms(),
            isolation: Some(mode.text().into()),
            ..Run::default()
        },
    )
}

/// Change the limit of a planned run (None removes it).
pub fn set_limit(db: &Db, agents: &Agents, id: i64, l: Option<Limit>) -> Result<()> {
    let r = get(db, id)?.context("no such run")?;
    anyhow::ensure!(
        r.status == PLANNED,
        "only a planned run can change its limit"
    );
    if let Some(l) = l {
        let a = agents
            .get(&r.agent)
            .with_context(|| format!("the agent '{}' is not enabled", r.agent))?;
        check_limit(db, a, l)?;
    }
    db.conn.execute(
        "UPDATE night_run SET limit_kind=?2, limit_value=?3 WHERE id=?1",
        params![id, l.map(|l| l.kind()), l.map(|l| l.value())],
    )?;
    Ok(())
}

/// Cancel a planned run. A run that is going is stopped by the scheduler, not here.
pub fn cancel(db: &Db, id: i64) -> Result<()> {
    let r = get(db, id)?.context("no such run")?;
    anyhow::ensure!(
        r.status == PLANNED,
        "run {id} is {}: only a planned run can be cancelled",
        r.status
    );
    // the status is checked again in the update: a starter may have claimed the run since the read
    let n = db.conn.execute(
        "UPDATE night_run SET status='skipped', why='cancelled', ended_at=?2
         WHERE id=?1 AND status IN ('planned')",
        params![id, now_ms()],
    )?;
    if n == 0 {
        let now = get(db, id)?.map_or_else(|| "gone".to_string(), |r| r.status);
        bail!("run {id} is now {now}: only a planned run can be cancelled");
    }
    Ok(())
}

/// The run's log, last `max` bytes, for `eye night log` and the board.
pub fn read_log(paths: &Paths, id: i64, max: usize) -> String {
    let b = std::fs::read(run_dir(paths, id).join("log")).unwrap_or_default();
    let from = b.len().saturating_sub(max);
    String::from_utf8_lossy(&b[from..]).into_owned()
}

/// Is `cwd` the folder of a night run of THIS eye home: `<night_dir>/<run id>/wt` or `/work`, or
/// below one? Both paths are canonicalized (a symlinked `EYE_HOME`, `/tmp` against `/private/tmp`),
/// so the answer does not depend on how either was spelled. A folder called `night/7/wt` anywhere
/// else is the user's own. `night_dir` is `<eye home>/night`; None (a database in memory) is never.
pub fn is_night_folder(night_dir: Option<&std::path::Path>, cwd: Option<&str>) -> bool {
    use std::path::{Path, PathBuf};
    let (Some(night), Some(cwd)) = (night_dir, cwd.filter(|c| !c.is_empty())) else {
        return false;
    };
    // the folder may be gone: canonicalize its nearest folder that is there, then add the rest
    fn real(p: &Path) -> PathBuf {
        let mut rest = vec![];
        let mut cur = p;
        loop {
            if let Ok(c) = cur.canonicalize() {
                return rest.iter().rev().fold(c, |acc, n| acc.join(n));
            }
            match (cur.parent(), cur.file_name()) {
                (Some(par), Some(n)) => {
                    rest.push(n.to_os_string());
                    cur = par;
                }
                _ => return p.to_path_buf(),
            }
        }
    }
    let (night, cwd) = (real(night), real(Path::new(cwd)));
    let Ok(rel) = cwd.strip_prefix(&night) else {
        return false;
    };
    let mut parts = rel.components().map(|c| c.as_os_str().to_string_lossy());
    let (Some(id), Some(dir)) = (parts.next(), parts.next()) else {
        return false;
    };
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && matches!(&*dir, "wt" | "work")
}

/// The slug of a branch name from a prompt or title: lowercase words joined by `-`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 32 {
            break;
        }
    }
    let t = out.trim_end_matches('-');
    if t.is_empty() {
        "run".into()
    } else {
        t.to_string()
    }
}
