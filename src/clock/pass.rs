//! The corrections pass: find the prompts nobody has judged yet, ask a model about the ones the
//! word filter lets through, and store every verdict. It runs once a night in the daemon and on
//! demand with `eye clock detect`. Newest first, so the days that matter most are done first; the
//! most that is sent in one pass is `NIGHTLY_MAX`, and what is left waits for the next one.

use super::detect::{self, Exchange, Judged};
use super::report::MAX_WEEKS;
use super::signals::{self, MAX_PROMPT_CHARS, UNJUDGED};
use crate::agents::Agents;
use crate::config::Config;
use crate::config_clock::ClockCfg;
use crate::conversation;
use crate::db::Db;
use crate::paths::Paths;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

/// Most exchanges sent in one pass.
pub const NIGHTLY_MAX: usize = 300;
const DAY_MS: i64 = 86_400_000;

/// What a pass did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Sessions whose conversation was read.
    pub sessions: usize,
    /// Exchanges sent to the model (not those it already judged once, or the filter dropped).
    pub sent: usize,
    pub corrections: usize,
    /// Prompts in the window that still have no verdict.
    pub pending: i64,
    /// Why the pass did not run or stopped early.
    pub stopped: Option<String>,
}

struct Candidate {
    ex: Exchange,
    /// The prompts with this exact exchange (a forked session repeats its parent's): one call
    /// answers for all of them.
    prompts: Vec<(String, i64)>,
}

/// Run one pass. `say` gets a short line after each call to the model.
pub fn run(
    db: &Db,
    paths: &Paths,
    cfg: &Config,
    agents: &Agents,
    now: i64,
    say: &mut dyn FnMut(&str),
) -> Result<Outcome> {
    let from = now - MAX_WEEKS * 7 * DAY_MS;
    let stop = |why: &str, db: &Db| -> Result<Outcome> {
        Ok(Outcome {
            pending: signals::pending(&db.conn, from)?,
            stopped: Some(why.into()),
            ..Outcome::default()
        })
    };
    if !ClockCfg::llm_on(cfg) {
        return stop("judging corrections is off ([clock] llm or [summary] enabled)", db);
    }
    let Some(ask) = agents.summary_agent(cfg).cloned() else {
        return stop("no agent is enabled to judge corrections", db);
    };
    let mut out = Outcome::default();
    let candidates = collect(db, paths, cfg, agents, from, now, &mut out)?;
    let work = paths.home.join("work");
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let model = model(cfg, ask.as_ref());
    let total = candidates.len();
    for batch in detect::batches(candidates.into_iter().map(|c| (c.prompts, c.ex)).collect()) {
        let refs: Vec<&Exchange> = batch.iter().map(|(_, e)| e).collect();
        let judged = match detect::judge(ask.as_ref(), &model, &work, &refs) {
            Ok(j) => j,
            Err(e) => {
                out.stopped = Some(format!("{e:#}"));
                break;
            }
        };
        let tx = db.conn.unchecked_transaction()?;
        for ((prompts, ex), j) in batch.iter().zip(judged) {
            let Some(j) = j else { continue };
            for (sid, ts) in prompts {
                out.corrections += usize::from(j.correction);
                store(&tx, sid, *ts, Some((&j, &model)), &ex.hash(), now)?;
            }
        }
        tx.commit()?;
        out.sent += batch.len();
        say(&format!(
            "judged {} of {total} exchanges, {} corrections so far",
            out.sent, out.corrections
        ));
    }
    out.pending = signals::pending(&db.conn, from)?;
    Ok(out)
}

/// The model that judges: `[clock] model` when set, else the one summaries use.
fn model(cfg: &Config, agent: &dyn crate::agents::Agent) -> String {
    crate::summarize::summary_model(cfg, agent)
}

/// Walk the unjudged prompts newest first and settle what needs no model: a prompt the word
/// filter drops, one whose text cannot be found, one in an excluded folder (never sent) and one
/// that matches an exchange judged before (a forked session repeats its parent's history) get a
/// verdict at once. The rest are returned, at most `NIGHTLY_MAX`, an exchange that several
/// prompts share only once.
fn collect(
    db: &Db,
    paths: &Paths,
    cfg: &Config,
    agents: &Agents,
    from: i64,
    now: i64,
    out: &mut Outcome,
) -> Result<Vec<Candidate>> {
    let mut by_session: Vec<(String, Vec<i64>)> = vec![];
    let mut st = db.conn.prepare(&format!(
        "SELECT t.session_id, t.ts FROM turn t WHERE {UNJUDGED} AND t.ts>=?1 ORDER BY t.ts DESC"
    ))?;
    let mut rows = st.query(params![from, MAX_PROMPT_CHARS])?;
    while let Some(r) = rows.next()? {
        let (sid, ts): (String, i64) = (r.get(0)?, r.get(1)?);
        match by_session.iter_mut().find(|(s, _)| *s == sid) {
            Some((_, v)) => v.push(ts),
            None => by_session.push((sid, vec![ts])),
        }
    }
    let mut found: Vec<Candidate> = vec![];
    for (sid, stamps) in by_session {
        if found.len() >= NIGHTLY_MAX {
            break;
        }
        let Some(s) = db.session_by_id(&sid)? else {
            continue;
        };
        let excluded = s.cwd.as_deref().is_some_and(|c| cfg.is_excluded(c));
        let Some(agent) = agents.of_session(&s) else {
            continue;
        };
        let messages = if excluded {
            vec![]
        } else {
            match conversation::read(&s, paths, agent.as_ref()) {
                Ok(m) => m,
                Err(_) => continue,
            }
        };
        out.sessions += 1;
        let tx = db.conn.unchecked_transaction()?;
        for ts in stamps {
            if found.len() >= NIGHTLY_MAX {
                break;
            }
            let ex = if excluded {
                None
            } else {
                detect::exchange(&messages, ts).filter(|e| detect::is_candidate(&e.prompt))
            };
            let Some(ex) = ex else {
                store(&tx, &sid, ts, None, "", now)?;
                continue;
            };
            if let Some(j) = known(&tx, &ex.hash())? {
                store(&tx, &sid, ts, Some((&j.0, &j.1)), &ex.hash(), now)?;
            } else if let Some(same) = found.iter_mut().find(|c| c.ex == ex) {
                same.prompts.push((sid.clone(), ts));
            } else {
                found.push(Candidate {
                    ex,
                    prompts: vec![(sid.clone(), ts)],
                });
            }
        }
        tx.commit()?;
    }
    Ok(found)
}

/// The verdict of an exchange judged before, with the model that judged it.
fn known(conn: &Connection, hash: &str) -> Result<Option<(Judged, String)>> {
    Ok(conn
        .query_row(
            "SELECT verdict, phrase, model FROM correction WHERE input_hash=?1 AND model IS NOT NULL LIMIT 1",
            [hash],
            |r| {
                let verdict: String = r.get(0)?;
                Ok((
                    Judged {
                        correction: verdict == "yes",
                        what: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    },
                    r.get(2)?,
                ))
            },
        )
        .optional()?)
}

/// Store a verdict. `judged` None is "not a correction, and no model was asked".
fn store(
    conn: &Connection,
    sid: &str,
    ts: i64,
    judged: Option<(&Judged, &str)>,
    hash: &str,
    now: i64,
) -> Result<()> {
    let (verdict, what, model) = match judged {
        Some((j, m)) => (if j.correction { "yes" } else { "no" }, j.what.as_str(), Some(m)),
        None => ("no", "", None),
    };
    conn.execute(
        "INSERT OR REPLACE INTO correction(session_id, prompt_ts, verdict, phrase, model, judged_at, input_hash)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![sid, ts, verdict, what, model, now, (!hash.is_empty()).then_some(hash)],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
