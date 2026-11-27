//! Sessions read before eye kept turn events have their cursors at the end of their files, so the
//! normal read never sees their past turns. The backfill reads the turns of those sessions from
//! the transcript, or from eye's archive copy when the original is gone, newest first, a few per
//! pass, and touches nothing else (no cursor, output, token count or summary). A session is done
//! once: the daemon does a few per tick, `eye scan` all of them, and `store::record` leaves a
//! session that is not done alone, so a pass that repeats or meets a normal read of the same
//! session adds nothing twice. A transcript that has grown since the scan last read it waits for
//! that scan: the back-fill reads to the end of the file, and the scan would read the same lines
//! again from its older cursor. Progress is the `turn_state` rows; when none is left the pass
//! writes `turns_backfilled_at` to `meta`.

use super::store;
use super::TurnEvent;
use crate::agents::Agents;
use crate::archive;
use crate::config::Config;
use crate::db::Db;
use crate::model::Session;
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::{params, TransactionBehavior};

/// Sessions the daemon reads per pass, so the board and the picker stay quick.
pub const BATCH: usize = 5;
/// A pass stops early after this long, so a few very long transcripts do not hold up the daemon.
const PASS_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);
/// A pass stops starting new sessions after reading this many bytes of transcripts (the first
/// session of a pass is always read, so a huge one still gets done, alone).
const PASS_BYTES: u64 = 32 << 20;
/// `meta` key set when no session is waiting any more.
pub const DONE_KEY: &str = "turns_backfilled_at";

/// What a pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
    pub counted: usize,
    pub unreadable: usize,
    /// Sessions still waiting for the next pass.
    pub left: usize,
}

/// The sessions to read: those of an enabled agent that have a transcript or a copy of one and no
/// turns yet (a session of an agent that is switched off waits until it is switched on).
const WAITING: &str = "FROM session s WHERE transcript_state IN ('present','archived')
    AND agent IN (SELECT value FROM json_each(?1))
    AND NOT EXISTS (SELECT 1 FROM turn_state w WHERE w.session_id=s.id)";

fn enabled_ids(agents: &Agents) -> String {
    serde_json::json!(agents.iter().map(|a| a.id()).collect::<Vec<_>>()).to_string()
}

/// Read up to `limit` sessions that have no turns yet, the most recently active first.
pub fn run_batch(db: &mut Db, paths: &Paths, cfg: &Config, limit: usize) -> Result<Pass> {
    if !cfg.clock.enabled {
        return Ok(Pass::default());
    }
    let agents = &crate::agents::enabled(cfg);
    let enabled = enabled_ids(agents);
    let ids: Vec<String> = db
        .conn
        .prepare(&format!(
            "SELECT id {WAITING} ORDER BY last_activity_at DESC"
        ))?
        .query_map(params![enabled], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut pass = Pass::default();
    let started = std::time::Instant::now();
    let mut read_bytes = 0u64;
    for id in &ids {
        if started.elapsed() > PASS_BUDGET
            || read_bytes >= PASS_BYTES
            || pass.counted + pass.unreadable >= limit
        {
            break;
        }
        let Some(s) = db.session_by_id(id)? else {
            continue;
        };
        read_bytes += s
            .transcript_path
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map_or(0, |m| m.len());
        // The file is read while the write lock is held: a scan in another process that appends
        // between this read and the store would otherwise be stored twice.
        let tx = db
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !store::is_counted(&tx, id)? {
            if s.cwd.as_deref().is_some_and(|c| cfg.is_excluded(c)) {
                // left out like the scan leaves it out, and done, so it is not asked for again
                store::record(&tx, id, &[], true)?;
                pass.counted += 1;
            } else if caught_up(&tx, &s, agents)? {
                match read(&s, paths, agents) {
                    Ok(turns) if caught_up(&tx, &s, agents)? => {
                        store::record(&tx, id, &turns, true)?;
                        pass.counted += 1;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        store::mark_unreadable(&tx, id)?;
                        pass.unreadable += 1;
                    }
                }
            }
        }
        tx.commit()?;
    }
    pass.left = waiting(db, agents)?;
    if pass.left == 0 && db.meta_get(DONE_KEY)?.is_none() {
        db.meta_set(DONE_KEY, &crate::util::now_ms().to_string())?;
    }
    Ok(pass)
}

/// Has the scan read the transcript as far as it goes now? A transcript with no stamp (gone, so
/// read from eye's copy) or no cursor has nothing a scan could read again.
fn caught_up(conn: &rusqlite::Connection, s: &Session, agents: &Agents) -> Result<bool> {
    let (Some(path), Some(agent)) = (s.transcript_path.as_deref(), agents.of_session(s)) else {
        return Ok(true);
    };
    let (Some(now), Some(read)) = (
        agent.transcript_stamp(std::path::Path::new(path)),
        crate::db::cursor_get(conn, path)?,
    ) else {
        return Ok(true);
    };
    Ok(now == read.stamp)
}

/// Read every session that has no turns yet, in batches of `BATCH`.
pub fn run_all(db: &mut Db, paths: &Paths, cfg: &Config) -> Result<Pass> {
    let mut total = Pass::default();
    loop {
        let pass = run_batch(db, paths, cfg, BATCH)?;
        total.counted += pass.counted;
        total.unreadable += pass.unreadable;
        total.left = pass.left;
        if pass.counted + pass.unreadable == 0 || pass.left == 0 {
            return Ok(total);
        }
    }
}

/// One batch in the daemon's usage thread, so the board and the picker stay quick. The line to log,
/// when there is something to say.
pub fn round(paths: &Paths, cfg: &Config) -> Option<String> {
    match Db::open(&paths.db_path()).and_then(|mut db| run_batch(&mut db, paths, cfg, BATCH)) {
        Ok(p) if p.counted + p.unreadable > 0 => Some(format!(
            "turn events: {} sessions counted, {} unreadable, {} to go",
            p.counted, p.unreadable, p.left
        )),
        Ok(_) => None,
        Err(e) => Some(format!("turn events: {e:#}")),
    }
}

/// How many sessions have no turns yet.
pub fn waiting(db: &Db, agents: &Agents) -> Result<usize> {
    let n: i64 = db.conn.query_row(
        &format!("SELECT count(*) {WAITING}"),
        [enabled_ids(agents)],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// The turns of a session, from its transcript or, when that is gone, eye's copy of it.
fn read(s: &Session, paths: &Paths, agents: &Agents) -> Result<Vec<TurnEvent>> {
    let agent = agents
        .of_session(s)
        .context("the agent of the session is not enabled")?;
    let readable = archive::readable(s, paths, agent.as_ref())?;
    agent.turn_events(&readable.path)
}
