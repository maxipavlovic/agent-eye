//! The once-a-minute check of the night shift: start planned runs when the window, the quiet rule
//! and `parallel` allow, watch the running ones (alive by pid and start time, limits), and finish
//! the ones that ended. Everything that touches the machine goes through [`Sys`], so tests set the
//! clock and the live sessions themselves.

use super::clock::{self, Window};
use super::{launch, Limit, Run, DONE, FAILED, LOST, PLANNED, RUNNING, STARTING, STOPPED};
use crate::agents::{origin, Agents, LiveSession};
use crate::config::Config;
use crate::db::Db;
use crate::loose::System;
use crate::paths::Paths;
use crate::quiet::Quiet;
use crate::util::{head_chars, now_ms};
use anyhow::{Context, Result};
use rusqlite::params;
use std::path::Path;

/// What finishing a run found out (the part that needs the machine).
#[derive(Debug, Clone, Default)]
pub struct Collected {
    /// The session the run made.
    pub session_id: Option<String>,
    /// Its last assistant message, masked.
    pub report: Option<String>,
    pub diff_stat: String,
}

/// The machine, as the scheduler sees it.
pub trait Sys {
    fn now(&self) -> i64;
    /// The offset from UTC of the local zone at a time, in ms.
    fn offset(&self, ts: i64) -> i64;
    /// Is a session you work in live: not a night run, busy or touched within `quiet_ms`?
    fn interactive_busy(&self, quiet_ms: i64) -> bool;
    /// Is the process there with this start time?
    fn alive(&self, pid: i64, start: Option<i64>) -> bool;
    /// SIGTERM to the process group of `pid`.
    fn terminate(&self, pid: i64);
    /// Make the folder and start the run.
    fn start(&self, run: &Run) -> Result<launch::Started>;
    /// The exit code the wrapper wrote, when it did.
    fn exit_code(&self, run: &Run) -> Option<i32>;
    /// The end of the run's log.
    fn log_tail(&self, run: &Run) -> String;
    /// Index what the run wrote, find its session, report and diff.
    fn collect(&self, db: &mut Db, run: &Run) -> Collected;
    /// The other enabled agent, when there is one.
    fn other_agent(&self, agent: &str) -> Option<String>;
    fn notify(&self, title: &str, body: &str);
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pass {
    pub started: Vec<i64>,
    pub finished: Vec<i64>,
    pub stopped: Vec<i64>,
}

/// Words in the end of a log that mean the plan is out.
pub fn rate_limited(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "rate limit",
        "rate-limit",
        "usage limit",
        "limit reached",
        "quota",
        "too many requests",
        "429",
        "hit your limit",
        "out of credits",
    ]
    .iter()
    .any(|w| t.contains(w))
}

fn limits_of(db: &Db, agent: &str) -> Option<crate::usage::PlanLimits> {
    crate::usage::limits::load(&db.conn, agent).ok().flatten()
}

/// Is this agent's plan used up (a live window at 100% or more)?
pub fn plan_out(db: &Db, agent: &str, now: i64) -> bool {
    limits_of(db, agent).is_some_and(|l| {
        l.windows
            .iter()
            .any(|w| w.used_percent >= 100.0 && w.resets_at.is_none_or(|r| r > now))
    })
}

/// Tokens a run's session used: input, output and cache writes, with its sub-agents. Cache reads
/// are left out (they are most of the traffic and not what a budget means).
pub fn run_tokens(db: &Db, session_id: &str) -> i64 {
    db.session_by_id(session_id)
        .ok()
        .flatten()
        .and_then(|s| s.tokens)
        .map_or(0, |t| {
            t.input + t.output + t.cache_write + t.subagents_total
        })
}

/// Why this running run must stop now, if it must.
pub fn limit_hit(db: &Db, run: &Run, now: i64) -> Option<String> {
    match run.limit()? {
        Limit::Minutes(m) => {
            let at = run.started_at?;
            (now - at >= m * 60_000).then(|| format!("time limit {m} min reached"))
        }
        Limit::Tokens(t) => {
            let used = run_tokens(db, run.run_session_id.as_deref()?);
            (used >= t).then(|| {
                format!(
                    "token budget {} reached ({} used)",
                    crate::util::fmt_tokens(t),
                    crate::util::fmt_tokens(used)
                )
            })
        }
        Limit::WeeklyPercent(p) => {
            let l = limits_of(db, &run.agent)?;
            let w = l
                .windows
                .iter()
                .filter(|w| super::is_weekly(w) && w.resets_at.is_none_or(|r| r > now))
                .find(|w| w.used_percent > p as f64)?;
            Some(format!(
                "weekly limit {p}% reached ({:.0}% used)",
                w.used_percent
            ))
        }
    }
}

fn set_status(db: &Db, id: i64, status: &str, why: Option<&str>) -> Result<()> {
    db.conn.execute(
        "UPDATE night_run SET status=?2, why=?3 WHERE id=?1",
        params![id, status, why],
    )?;
    Ok(())
}

/// The session of a run that started in `dir` (the agent chose its id).
fn link_session(db: &Db, run: &Run) -> Result<()> {
    let (None, Some(wt)) = (&run.run_session_id, &run.worktree) else {
        return Ok(());
    };
    let found: Option<String> = db
        .conn
        .query_row(
            "SELECT id FROM session WHERE agent=?3 AND (cwd=?1 OR cwd LIKE ?2) ORDER BY started_at DESC LIMIT 1",
            params![wt, format!("{}/%", wt.trim_end_matches('/')), run.agent],
            |r| r.get(0),
        )
        .ok();
    if let Some(id) = found {
        db.conn.execute(
            "UPDATE night_run SET run_session_id=?2 WHERE id=?1",
            params![run.id, id],
        )?;
    }
    Ok(())
}

/// A run's process is gone: store what it did and put its session where Max will look.
fn finish(db: &mut Db, sys: &dyn Sys, run: &Run) -> Result<()> {
    let code = sys.exit_code(run);
    let col = sys.collect(db, run);
    let tail = sys.log_tail(run);
    let (status, why) = if run.status == STOPPED {
        (STOPPED, run.why.clone())
    } else {
        match code {
            Some(0) => (DONE, None),
            Some(c) => {
                let last = tail
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                (FAILED, Some(format!("exit {c}: {}", head_chars(last, 200))))
            }
            None => (
                FAILED,
                Some("the process ended without an exit code".into()),
            ),
        }
    };
    // out of plan: the same idea goes on in the other agent, when the run allows it
    let mut why = why;
    if status == FAILED && run.kind == "idea" && run.switch_ok && rate_limited(&tail) {
        if let Some(other) = sys.other_agent(&run.agent) {
            db.conn.execute(
                "INSERT INTO night_run(kind, idea_id, agent, switch_ok, folder, prompt, night, status, limit_kind, limit_value, created_at)
                 VALUES('idea',?1,?2,0,?3,?4,?5,'planned',?6,?7,?8)",
                params![run.idea_id, other, run.folder, run.prompt, run.night,
                        run.limit_kind.as_deref().filter(|k| *k != "weekly"), run.limit_value.filter(|_| run.limit_kind.as_deref() != Some("weekly")), now_ms()],
            )?;
            why = Some(format!(
                "{} ran out of plan; started again on {other} as run {}",
                run.agent,
                db.conn.last_insert_rowid()
            ));
        }
    }
    db.conn.execute(
        "UPDATE night_run SET status=?2, why=?3, ended_at=?4, report=?5, diff_stat=?6,
                run_session_id=COALESCE(?7, run_session_id) WHERE id=?1",
        params![
            run.id,
            status,
            why,
            sys.now(),
            col.report,
            col.diff_stat,
            col.session_id
        ],
    )?;
    if let Some(sid) = col.session_id.as_deref().or(run.run_session_id.as_deref()) {
        let _ = crate::mark::mark(db, sid, "needs-you", Some(super::NEXT_STEP));
        // a done idea leaves Ideas for its session, as a started one does (DISCARD gives it back)
        if let (DONE, Some(idea)) = (status, run.idea_id.filter(|_| run.kind == "idea")) {
            db.conn.execute(
                "UPDATE idea SET session_id=?2 WHERE id=?1 AND session_id IS NULL",
                params![idea, sid],
            )?;
        }
    }
    Ok(())
}

/// A run left in `starting` this long was abandoned (a worktree add or a clone is slow, not this slow).
const STARTING_STALE_MS: i64 = 30 * 60_000;

/// One pass: finish, watch, start.
pub fn tick(db: &mut Db, sys: &dyn Sys, cfg: &Config) -> Result<Pass> {
    tick_with(db, sys, cfg, true)
}

/// [`tick`], with `start` false to only watch and finish what is already going.
pub fn tick_with(db: &mut Db, sys: &dyn Sys, cfg: &Config, start: bool) -> Result<Pass> {
    let mut pass = Pass::default();
    let now = sys.now();
    // 1. the running ones: gone? over a limit?
    for run in super::with_status(db, RUNNING)? {
        let alive = run.pid.is_some_and(|p| sys.alive(p, run.pid_start));
        if !alive {
            finish(db, sys, &run)?;
            pass.finished.push(run.id);
            continue;
        }
        link_session(db, &run)?;
        let run = super::get(db, run.id)?.context("run vanished")?;
        if let Some(why) = limit_hit(db, &run, now) {
            // never signal a pid whose start time is unknown: it may be another process now
            if let (Some(pid), Some(_)) = (run.pid, run.pid_start) {
                sys.terminate(pid);
            }
            set_status(db, run.id, STOPPED, Some(&why))?;
            pass.stopped.push(run.id);
        }
    }
    // 1b. a run claimed long ago and never recorded: the starter died while launching
    for run in super::with_status(db, STARTING)? {
        if run.started_at.is_some_and(|t| now - t > STARTING_STALE_MS) {
            db.conn.execute(
                "UPDATE night_run SET status='failed', why='the start was interrupted', ended_at=?2
                 WHERE id=?1 AND status='starting'",
                params![run.id, now],
            )?;
        }
    }
    // 2. stopped ones that still have a process: finish when it is gone
    for run in super::with_status(db, STOPPED)? {
        if run.ended_at.is_some() {
            continue;
        }
        if !run.pid.is_some_and(|p| sys.alive(p, run.pid_start)) {
            finish(db, sys, &run)?;
            pass.finished.push(run.id);
        }
    }
    // 3. start planned ones
    if start && cfg.night.enabled {
        start_due(db, sys, cfg, now, &mut pass)?;
    }
    if !pass.finished.is_empty() {
        let n = pass.finished.len();
        sys.notify(
            "Night shift",
            &format!("{n} run{} finished", if n == 1 { "" } else { "s" }),
        );
    }
    Ok(pass)
}

fn running_count(db: &Db) -> Result<usize> {
    Ok(db.conn.query_row(
        "SELECT COUNT(*) FROM night_run WHERE status IN ('running','starting') OR (status='stopped' AND ended_at IS NULL)",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

fn start_due(db: &mut Db, sys: &dyn Sys, cfg: &Config, now: i64, pass: &mut Pass) -> Result<()> {
    let w = Window::parse(&cfg.night.window).map_err(anyhow::Error::msg)?;
    let off = |t: i64| sys.offset(t);
    if !clock::in_window(&w, now, &off) {
        return Ok(());
    }
    let tonight = clock::night_of(&w, now, &off);
    let due: Vec<Run> = super::with_status(db, PLANNED)?
        .into_iter()
        .filter(|r| r.night <= tonight)
        .collect();
    if due.is_empty() {
        return Ok(());
    }
    if sys.interactive_busy(i64::from(cfg.night.quiet_minutes) * 60_000) {
        return Ok(());
    }
    let room = (cfg.night.parallel as usize).saturating_sub(running_count(db)?);
    for run in due.into_iter().take(room) {
        if start_run(db, sys, run.clone(), true)? {
            pass.started.push(run.id);
        }
    }
    Ok(())
}

/// Start one planned run now. With `allow_switch`, a run whose agent's plan is out starts on the
/// other agent when it allows that. Returns whether it started.
pub fn start_run(db: &Db, sys: &dyn Sys, mut run: Run, allow_switch: bool) -> Result<bool> {
    let now = sys.now();
    // Claim the run before anything is made: a second starter, or a cancel, that comes later finds
    // it no longer planned and does nothing.
    let claimed = db.conn.execute(
        "UPDATE night_run SET status='starting', started_at=?2 WHERE id=?1 AND status='planned'",
        params![run.id, now],
    )?;
    if claimed == 0 {
        return Ok(false);
    }
    if allow_switch && run.kind == "idea" && run.switch_ok && plan_out(db, &run.agent, now) {
        if let Some(other) = sys
            .other_agent(&run.agent)
            .filter(|o| !plan_out(db, o, now))
        {
            db.conn.execute(
                "UPDATE night_run SET agent=?2, why=?3 WHERE id=?1 AND status='starting'",
                params![
                    run.id,
                    other,
                    format!("{} plan out; started on {other}", run.agent)
                ],
            )?;
            run.agent = other;
            // a weekly limit was about the other plan
            if run.limit_kind.as_deref() == Some("weekly") {
                db.conn.execute(
                    "UPDATE night_run SET limit_kind=NULL, limit_value=NULL WHERE id=?1",
                    [run.id],
                )?;
                run.limit_kind = None;
                run.limit_value = None;
            }
        }
    }
    match sys.start(&run) {
        Ok(s) => {
            // Without the start time a pid cannot be told from a reused one, so eye could never
            // check the process or signal it safely: the run is lost, with its pid and folder kept.
            let (status, why) = if s.pid_start.is_some() {
                (RUNNING, None)
            } else {
                (
                    LOST,
                    Some("started, but its process start time could not be read: eye cannot follow or stop it; look in its folder"),
                )
            };
            let n = db.conn.execute(
                "UPDATE night_run SET status=?9, why=?10, pid=?2, pid_start=?3, worktree=?4, branch=?5,
                        base_ref=?6, run_session_id=?7, started_at=?8,
                        ended_at=CASE WHEN ?9='lost' THEN ?8 ELSE NULL END
                 WHERE id=?1 AND status='starting'",
                params![
                    run.id,
                    s.pid,
                    s.pid_start,
                    s.iso.root.to_string_lossy(),
                    s.iso.branch,
                    s.iso.base_ref,
                    s.session_id,
                    sys.now(),
                    status,
                    why
                ],
            )?;
            if n == 0 {
                // the claim was taken back while it launched: do not leave an unwatched process
                if s.pid_start.is_some() {
                    sys.terminate(s.pid);
                }
                return Ok(false);
            }
            Ok(status == RUNNING)
        }
        Err(e) => {
            db.conn.execute(
                "UPDATE night_run SET status='failed', why=?2, ended_at=?3 WHERE id=?1 AND status='starting'",
                params![run.id, format!("could not start: {e:#}"), sys.now()],
            )?;
            Ok(false)
        }
    }
}

/// Does a live session of `agent` mean you are at work? Not when it is gone, a script's (`sdk-cli`),
/// a night run's, or of an origin `[board] quiet` keeps quiet; otherwise while it is busy or was
/// touched within `quiet_ms`.
pub(crate) fn is_your_work(
    agent: &str,
    s: &LiveSession,
    night: &Path,
    quiet: Quiet,
    now: i64,
    quiet_ms: i64,
) -> bool {
    let origin = s
        .entrypoint
        .as_deref()
        .and_then(|e| origin::classify(agent, e));
    s.alive
        && s.entrypoint.as_deref() != Some("sdk-cli")
        && !quiet.holds(origin)
        && !super::is_night_folder(Some(night), s.cwd.as_deref())
        && (s.status.as_deref() == Some("busy") || s.updated_at.is_some_and(|u| now - u < quiet_ms))
}

/// The machine for real.
pub struct RealSys<'a> {
    pub paths: &'a Paths,
    pub cfg: &'a Config,
    pub agents: &'a Agents,
    pub probe: System,
}

impl<'a> RealSys<'a> {
    pub fn new(paths: &'a Paths, cfg: &'a Config, agents: &'a Agents) -> Self {
        RealSys {
            paths,
            cfg,
            agents,
            probe: System::new(),
        }
    }

    fn night_dir(&self) -> std::path::PathBuf {
        self.paths.home.join("night")
    }
}

impl Sys for RealSys<'_> {
    fn now(&self) -> i64 {
        now_ms()
    }

    fn offset(&self, ts: i64) -> i64 {
        clock::local_offset(ts)
    }

    fn interactive_busy(&self, quiet_ms: i64) -> bool {
        let now = now_ms();
        let night = self.night_dir();
        let quiet = Quiet::of(self.cfg);
        self.agents.iter().any(|a| {
            a.live_sessions()
                .iter()
                .any(|s| is_your_work(a.id(), s, &night, quiet, now, quiet_ms))
        })
    }

    fn alive(&self, pid: i64, start: Option<i64>) -> bool {
        let Ok(p) = u32::try_from(pid) else {
            return false;
        };
        match (crate::platform::procs::info(p), start) {
            (Some(i), Some(s)) => i.start == s,
            (Some(_), None) => crate::util::pid_alive(pid),
            (None, _) => false,
        }
    }

    fn terminate(&self, pid: i64) {
        if pid > 1 {
            if let Ok(p) = libc::pid_t::try_from(pid) {
                // SAFETY: kill with a process group (negative pid) and SIGTERM touches no memory.
                unsafe { libc::kill(-p, libc::SIGTERM) };
            }
        }
    }

    fn start(&self, run: &Run) -> Result<launch::Started> {
        let db = Db::open(&self.paths.db_path())?;
        launch::start(&self.probe, &db, self.paths, self.cfg, self.agents, run)
    }

    fn exit_code(&self, run: &Run) -> Option<i32> {
        std::fs::read_to_string(super::run_dir(self.paths, run.id).join("exit"))
            .ok()?
            .trim()
            .parse()
            .ok()
    }

    fn log_tail(&self, run: &Run) -> String {
        super::read_log(self.paths, run.id, 4000)
    }

    fn collect(&self, db: &mut Db, run: &Run) -> Collected {
        let _ = crate::scan::scan(db, self.cfg, false);
        let _ = link_session(db, run);
        let sid = super::get(db, run.id)
            .ok()
            .flatten()
            .and_then(|r| r.run_session_id);
        let report = sid
            .as_deref()
            .and_then(|s| db.session_by_id(s).ok().flatten())
            .and_then(|s| s.last_assistant_text)
            .map(|t| head_chars(&crate::summarize::mask_secrets(t.trim()), 6000))
            .filter(|t| !t.is_empty());
        let diff_stat = super::iso::diff_stat(&self.probe, run);
        if self.cfg.loose.enabled && sid.is_some() {
            let _ = crate::loose::scan_and_store(db, self.cfg, self.agents, now_ms());
        }
        Collected {
            session_id: sid,
            report,
            diff_stat,
        }
    }

    fn other_agent(&self, agent: &str) -> Option<String> {
        self.agents
            .iter()
            .map(|a| a.id().to_string())
            .find(|id| id != agent)
    }

    fn notify(&self, title: &str, body: &str) {
        if crate::remind::notices::is_on(self.paths, self.cfg.remind.notify) {
            let _ = crate::platform::notify::send(title, body);
        }
    }
}
