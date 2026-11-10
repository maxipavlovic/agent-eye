//! Live status and background jobs: who is open right now, and what the agent runs in the background.

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

type Want = HashMap<String, (Option<String>, Option<i64>)>;

/// For an agent whose live status cannot see a permission prompt: a session a hook recorded a
/// prompt for is `waiting`, not `busy`, until a hook says the prompt was answered.
fn show_prompts(tx: &Connection, agent: &dyn Agent, want: &mut Want) -> Result<()> {
    let mut st = tx.prepare(
        "SELECT id FROM session WHERE agent=?1 AND waiting_since IS NOT NULL AND waiting_prompt=1",
    )?;
    for id in st.query_map([agent.id()], |r| r.get::<_, String>(0))? {
        if let Some((status, _)) = want.get_mut(&id?) {
            if status.as_deref() == Some("busy") {
                *status = Some("waiting".into());
            }
        }
    }
    Ok(())
}

/// Copy live status from `sessions/<pid>.json` onto sessions that have a transcript. Entries
/// without a transcript (eye's own summarizer, `sdk-cli` runs that keep no session) are
/// ignored: they never become cards. Writes only what changed.
pub(super) fn apply_live(tx: &Connection, agent: &dyn Agent) -> Result<()> {
    let mut want = Want::new();
    for s in agent.live_sessions() {
        if s.alive {
            want.insert(s.session_id, (s.status, s.updated_at));
        }
    }
    if !agent.live_status_shows_prompts() {
        show_prompts(tx, agent, &mut want)?;
    }
    type LiveRow = (String, Option<String>, Option<i64>);
    let mut have: Vec<LiveRow> = vec![];
    {
        let mut st = tx.prepare(
            "SELECT id, live_status, live_updated_at FROM session
             WHERE agent=?1 AND (live_status IS NOT NULL OR live_updated_at IS NOT NULL)",
        )?;
        let rows = st.query_map([agent.id()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        for r in rows {
            have.push(r?);
        }
    }
    for (id, ls, lu) in &have {
        match want.get(id) {
            Some((s, u)) if s == ls && u == lu => {}
            Some(_) => {}
            None => {
                tx.execute(
                    "UPDATE session SET live_status=NULL, live_updated_at=NULL WHERE id=?1",
                    [id],
                )?;
            }
        }
    }
    for (id, (status, updated)) in &want {
        let same = have
            .iter()
            .any(|(i, s, u)| i == id && s == status && u == updated);
        if same {
            continue;
        }
        tx.execute(
            "UPDATE session SET live_status=?2, live_updated_at=?3, ended_at=NULL
             WHERE id=?1 AND transcript_state='present'",
            params![id, status, updated],
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
