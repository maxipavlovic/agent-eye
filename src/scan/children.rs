//! Sub-agents of a session: one child stream each, and their token use summed into the session.

use crate::agents::{Agent, LastRead, SubAgents, Transcript};
use crate::db::{cursor_get, cursor_set, meta_set};
use crate::model::Tokens;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Read the sub-agents of the session in `f` (when its row exists): child streams and tokens.
pub(super) fn scan(tx: &Connection, agent: &dyn Agent, f: &Transcript, full: bool) -> Result<()> {
    let sub = agent.subagents(f);
    if sub.children.is_empty() && sub.transcripts.is_empty() {
        return Ok(());
    }
    let exists: bool = tx
        .query_row("SELECT 1 FROM session WHERE id=?1", [&f.session_id], |_| {
            Ok(true)
        })
        .optional()?
        .unwrap_or(false);
    if !exists {
        return Ok(());
    }
    tokens(tx, agent, &f.session_id, &sub, full)?;
    for c in &sub.children {
        tx.execute(
            "INSERT INTO stream(session_id,kind,key,title,status,detail,link,sort,updated_at)
             VALUES(?1,'child',?2,?3,?4,?5,NULL,0,?6)
             ON CONFLICT(session_id,kind,key) DO UPDATE SET title=excluded.title, status=excluded.status,
               detail=excluded.detail, updated_at=excluded.updated_at
             WHERE status IS NOT excluded.status OR title IS NOT excluded.title",
            params![
                f.session_id,
                c.id,
                c.title,
                if c.finished { "done" } else { "running" },
                c.detail,
                c.updated_at
            ],
        )?;
    }
    Ok(())
}

/// Sum the token use of the sub-agent transcripts into `session.tok_sub`. Each one keeps its own
/// cursor and counts (`subtok:<path>` in meta), so only what is new is read again.
fn tokens(
    tx: &Connection,
    agent: &dyn Agent,
    sid: &str,
    sub: &SubAgents,
    full: bool,
) -> Result<()> {
    #[derive(Default, Serialize, Deserialize)]
    struct Acc {
        tokens: Tokens,
        last_id: Option<String>,
    }
    let mut sum = 0i64;
    let mut any = false;
    for src in &sub.transcripts {
        any = true;
        let key = src.path.to_string_lossy().into_owned();
        let mkey = format!("subtok:{key}");
        let cursor = cursor_get(tx, &key)?;
        let mut acc: Acc = tx
            .query_row("SELECT value FROM meta WHERE key=?1", [&mkey], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|v| serde_json::from_str(&v).ok())
            .unwrap_or_default();
        let unchanged = cursor.as_ref().is_some_and(|c| c.stamp == src.stamp);
        if full || !unchanged {
            let since = cursor.as_ref().filter(|_| !full).map(|c| LastRead {
                marker: &c.marker,
                stamp: c.stamp,
            });
            if since.is_none() {
                acc = Acc::default();
            }
            let read = match agent.read_usage(src, since, acc.last_id.take()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("warn: {e:#}");
                    continue;
                }
            };
            if read.replaces {
                acc = Acc::default();
            }
            acc.tokens.add(&read.tokens);
            acc.last_id = read.last_id;
            meta_set(tx, &mkey, &serde_json::to_string(&acc)?)?;
            cursor_set(tx, &key, src.stamp, &read.marker)?;
        }
        sum += acc.tokens.main_total();
    }
    if any {
        tx.execute(
            "UPDATE session SET tok_sub=?2 WHERE id=?1",
            params![sid, sum],
        )?;
    } else {
        tx.execute(
            "UPDATE session SET tok_sub=NULL WHERE id=?1 AND tok_sub IS NOT NULL",
            [sid],
        )?;
    }
    Ok(())
}
