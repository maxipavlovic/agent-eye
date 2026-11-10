//! Sessions whose transcripts are gone but whose prompts survive in the agent's history file:
//! imported as history-only sessions (searchable, not resumable).

use super::transcript::prompt_title;
use crate::agents::{Agent, LastRead, PROMPTS_CAP, REPLIES_CAP};
use crate::config::Config;
use crate::db::{cursor_set, fts_write, Db};
use crate::util::head_chars;
use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use std::collections::{BTreeMap, HashMap, HashSet};

struct HistEx {
    kind: Option<String>,
    prompts: i64,
    started: Option<i64>,
    last: Option<i64>,
    first_prompt: Option<String>,
}

struct Hist {
    project: String,
    first: i64,
    last: i64,
    prompts: Vec<String>,
}

pub(super) fn import(
    db: &mut Db,
    agent: &dyn Agent,
    cfg: &Config,
    transcript_ids: &HashSet<String>,
    full: bool,
    repo_cache: &mut HashMap<String, Option<String>>,
) -> Result<u64> {
    let Some(src) = agent.history_source() else {
        return Ok(0);
    };
    let key = src.path.to_string_lossy().into_owned();
    let cursor = db.cursor_get(&key)?;
    if !full && cursor.as_ref().is_some_and(|c| c.stamp == src.stamp) {
        return Ok(0);
    }
    let since = cursor.as_ref().filter(|_| !full).map(|c| LastRead {
        marker: &c.marker,
        stamp: c.stamp,
    });
    let read = agent
        .read_history(&src, since)
        .with_context(|| format!("reading {}", src.path.display()))?;
    let reset = since.is_none() || read.replaces;
    let mut agg: BTreeMap<String, Hist> = BTreeMap::new();
    let mut n = 0u64;
    for prompt in read.prompts {
        let text = prompt.text.trim();
        if text.is_empty() {
            continue;
        }
        let ts = prompt.ts;
        let h = agg.entry(prompt.session_id).or_insert_with(|| Hist {
            project: prompt.project,
            first: ts,
            last: ts,
            prompts: vec![],
        });
        h.first = h.first.min(ts);
        h.last = h.last.max(ts);
        h.prompts.push(head_chars(text, 4_000));
        n += 1;
    }
    let tx = db
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    for (sid, h) in &agg {
        if transcript_ids.contains(sid) || cfg.is_excluded(&h.project) {
            continue;
        }
        let ex: Option<HistEx> = tx
            .query_row(
                "SELECT kind, prompt_count, started_at, last_activity_at, first_prompt FROM session WHERE id=?1",
                [sid],
                |r| {
                    Ok(HistEx {
                        kind: r.get(0)?,
                        prompts: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        started: r.get(2)?,
                        last: r.get(3)?,
                        first_prompt: r.get(4)?,
                    })
                },
            )
            .optional()?;
        if ex
            .as_ref()
            .is_some_and(|e| e.kind.as_deref() != Some("history_only"))
        {
            continue;
        }
        let (prev_n, prev_start, prev_last, prev_first) = match (&ex, reset) {
            (Some(e), false) => (e.prompts, e.started, e.last, e.first_prompt.clone()),
            _ => (0, None, None, None),
        };
        let first_prompt = prev_first.or_else(|| h.prompts.first().cloned());
        let title = first_prompt.as_deref().map(prompt_title);
        let root = super::repo_root(&h.project, repo_cache);
        tx.execute(
            "INSERT INTO session(id, agent) VALUES(?1, ?2) ON CONFLICT(id) DO NOTHING",
            params![sid, agent.id()],
        )?;
        tx.execute(
            "UPDATE session SET cwd=?2, repo_root=?3, title=?4, title_source='prompt', first_prompt=?5,
                started_at=?6, last_activity_at=?7, kind='history_only', transcript_state='deleted',
                prompt_count=?8, reply_count=0, last_user_text=?9
             WHERE id=?1",
            params![
                sid,
                h.project,
                root,
                title,
                first_prompt.as_deref().map(|p| head_chars(p, 2_000)),
                prev_start.map_or(h.first, |p| p.min(h.first)),
                prev_last.map_or(h.last, |p| p.max(h.last)),
                prev_n + h.prompts.len() as i64,
                h.prompts.last().map(|p| head_chars(p, 1_500)),
            ],
        )?;
        let joined = h.prompts.join("\n") + "\n";
        fts_write(&tx, sid, &joined, "", reset, PROMPTS_CAP, REPLIES_CAP)?;
    }
    cursor_set(&tx, &key, src.stamp, &read.marker)?;
    tx.commit()?;
    Ok(n)
}
