//! Live status and background jobs: who is open right now, and what the agent runs in the background.

use crate::activity::to_column;
use crate::agents::{Agent, Job, PROMPTS_CAP, REPLIES_CAP};
use crate::db::fts_write;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};

/// Mark sessions started as background jobs, take their note and name, and make a job the child
/// stream of the session that started it (`hits` pairs a parent session with a job index).
pub(super) fn apply_jobs(
    tx: &Connection,
    agent: &dyn Agent,
    jobs: &[Job],
    hits: &[(String, usize)],
) -> Result<()> {
    let exists = |id: &str| -> Result<bool> {
        Ok(tx
            .query_row("SELECT 1 FROM session WHERE id=?1", [id], |_| Ok(true))
            .optional()?
            .unwrap_or(false))
    };
    let mut job_sid: Vec<Option<String>> = vec![];
    for j in jobs {
        let mut found = None;
        for c in [&j.session_id, &j.resume_session_id].into_iter().flatten() {
            if exists(c)? {
                found = Some(c.clone());
                break;
            }
        }
        job_sid.push(found);
    }
    let mut touched: HashSet<String> = HashSet::new();
    for (j, sid) in jobs.iter().zip(&job_sid) {
        let Some(sid) = sid else { continue };
        tx.execute(
            "UPDATE session SET kind='bg' WHERE id=?1 AND kind IS NOT 'bg'",
            [sid],
        )?;
        if let (Some(detail), Some(up)) = (&j.detail, j.updated_at) {
            let detail = agent.clean_recap(detail);
            let n = tx.execute(
                "UPDATE session SET recap=?2, recap_at=?3 WHERE id=?1 AND (recap_at IS NULL OR recap_at<?3) AND recap IS NOT ?2",
                params![sid, detail, up],
            )?;
            if n > 0 {
                touched.insert(sid.clone());
            }
        }
        if let Some(name) = &j.name {
            let n = tx.execute(
                "UPDATE session SET title=?2, title_source='agent' WHERE id=?1 AND (title_source IS NULL OR title_source='prompt')",
                params![sid, name],
            )?;
            if n > 0 {
                touched.insert(sid.clone());
            }
        }
    }
    for (parent, ji) in hits {
        let Some(Some(sid)) = job_sid.get(*ji) else {
            continue;
        };
        if sid == parent {
            continue;
        }
        let parent_started: Option<i64> = tx
            .query_row(
                "SELECT started_at FROM session WHERE id=?1",
                [parent],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let (Some(ps), Some(ca)) = (parent_started, jobs[*ji].created_at) {
            if ps > ca {
                continue;
            }
        }
        tx.execute(
            "UPDATE session SET parent_session_id=?1 WHERE id=?2 AND parent_session_id IS NULL",
            params![parent, sid],
        )?;
    }
    for (j, sid) in jobs.iter().zip(&job_sid) {
        let Some(sid) = sid else { continue };
        let parent: Option<String> = tx
            .query_row(
                "SELECT parent_session_id FROM session WHERE id=?1",
                [sid],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(parent) = parent else { continue };
        let title = j.name.clone().unwrap_or_else(|| j.short.clone());
        let n = tx.execute(
            "INSERT INTO stream(session_id,kind,key,title,status,detail,link,sort,updated_at)
             VALUES(?1,'child',?2,?3,?4,?5,?2,1,?6)
             ON CONFLICT(session_id,kind,key) DO UPDATE SET title=excluded.title, status=excluded.status,
               detail=excluded.detail, updated_at=excluded.updated_at
             WHERE status IS NOT excluded.status OR title IS NOT excluded.title OR detail IS NOT excluded.detail",
            params![parent, sid, title, j.state, j.detail, j.updated_at],
        )?;
        if n > 0 {
            touched.insert(sid.clone());
        }
    }
    for sid in touched {
        fts_write(tx, &sid, "", "", false, PROMPTS_CAP, REPLIES_CAP)?;
    }
    Ok(())
}

/// What the live state says about one open session.
#[derive(Debug, Clone, PartialEq)]
struct Live {
    status: Option<String>,
    updated: Option<i64>,
    wait: Option<String>,
    remote: Option<String>,
    loops: Option<String>,
}

type Want = HashMap<String, Live>;

/// For an agent whose live status cannot see a permission prompt: a session a hook recorded a
/// prompt for is `waiting`, not `busy`, until a hook says the prompt was answered.
fn show_prompts(tx: &Connection, agent: &dyn Agent, want: &mut Want) -> Result<()> {
    let mut st = tx.prepare(
        "SELECT id FROM session WHERE agent=?1 AND waiting_since IS NOT NULL AND waiting_prompt=1",
    )?;
    for id in st.query_map([agent.id()], |r| r.get::<_, String>(0))? {
        if let Some(live) = want.get_mut(&id?) {
            if live.status.as_deref() == Some("busy") {
                live.status = Some("waiting".into());
            }
        }
    }
    Ok(())
}

/// Copy live status from `sessions/<pid>.json` onto sessions that have a transcript, with why it
/// waits, its remote control link and the loops it shows. Entries without a transcript (eye's own
/// summarizer, `sdk-cli` runs that keep no session) are ignored: they never become cards. Writes
/// only what changed.
pub(super) fn apply_live(tx: &Connection, agent: &dyn Agent) -> Result<()> {
    let mut want = Want::new();
    for s in agent.live_sessions() {
        if s.alive {
            want.insert(
                s.session_id,
                Live {
                    status: s.status,
                    updated: s.updated_at,
                    wait: s.waiting_for,
                    remote: s.remote_url,
                    loops: to_column(&s.loops),
                },
            );
        }
    }
    if !agent.live_status_shows_prompts() {
        show_prompts(tx, agent, &mut want)?;
    }
    let mut have: HashMap<String, Live> = HashMap::new();
    {
        let mut st = tx.prepare(
            "SELECT id, live_status, live_updated_at, live_wait, remote_url, live_loops FROM session
             WHERE agent=?1 AND (live_status IS NOT NULL OR live_updated_at IS NOT NULL)",
        )?;
        let rows = st.query_map([agent.id()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Live {
                    status: r.get(1)?,
                    updated: r.get(2)?,
                    wait: r.get(3)?,
                    remote: r.get(4)?,
                    loops: r.get(5)?,
                },
            ))
        })?;
        for r in rows {
            let (id, live) = r?;
            have.insert(id, live);
        }
    }
    for id in have.keys().filter(|id| !want.contains_key(*id)) {
        tx.execute(
            "UPDATE session SET live_status=NULL, live_updated_at=NULL, live_wait=NULL, remote_url=NULL,
               live_loops=NULL WHERE id=?1",
            [id],
        )?;
    }
    for (id, live) in &want {
        if have.get(id) == Some(live) {
            continue;
        }
        tx.execute(
            "UPDATE session SET live_status=?2, live_updated_at=?3, live_wait=?4, remote_url=?5, live_loops=?6,
               ended_at=NULL WHERE id=?1 AND transcript_state='present'",
            params![
                id,
                live.status,
                live.updated,
                live.wait,
                live.remote,
                live.loops
            ],
        )?;
    }
    Ok(())
}

/// A session with no live status has ended at its last activity.
pub(super) fn close_ended(tx: &Connection) -> Result<()> {
    tx.execute(
        "UPDATE session SET ended_at=last_activity_at WHERE live_status IS NULL AND ended_at IS NULL",
        [],
    )?;
    Ok(())
}
