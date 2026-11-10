//! Storing what a transcript parse found: the session row, tokens, outputs and search text.

use crate::agents::{Agent, Delta, Transcript, PROMPTS_CAP, REPLIES_CAP};
use crate::config::Config;
use crate::db::{fts_write, meta_set};
use crate::util::{head_chars, now_ms};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
struct Ex {
    title: Option<String>,
    title_source: Option<String>,
    recap_at: Option<i64>,
    prompt_count: i64,
    reply_count: i64,
    first_prompt: Option<String>,
    started_at: Option<i64>,
    last_activity_at: Option<i64>,
    kind: Option<String>,
    cwd: Option<String>,
    transcript_state: Option<String>,
}

fn title_rank(s: &str) -> u8 {
    match s {
        "custom" => 4,
        "ai" => 3,
        "agent" => 2,
        "prompt" => 1,
        _ => 0,
    }
}

pub(super) fn prompt_title(p: &str) -> String {
    head_chars(&p.split_whitespace().collect::<Vec<_>>().join(" "), 80)
}

/// Store what one parse of a transcript found: the session row, its tokens, outputs and search text.
pub(super) fn apply(
    tx: &Connection,
    agent: &dyn Agent,
    f: &Transcript,
    d: &Delta,
    reset_in: bool,
    cfg: &Config,
    repo_cache: &mut HashMap<String, Option<String>>,
) -> Result<()> {
    let sid = f.session_id.as_str();
    let found: Option<Ex> = tx
        .query_row(
            "SELECT title, title_source, recap_at, prompt_count, reply_count, first_prompt, started_at,
                    last_activity_at, kind, cwd, transcript_state FROM session WHERE id=?1",
            [sid],
            |r| {
                Ok(Ex {
                    title: r.get(0)?,
                    title_source: r.get(1)?,
                    recap_at: r.get(2)?,
                    prompt_count: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                    reply_count: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    first_prompt: r.get(5)?,
                    started_at: r.get(6)?,
                    last_activity_at: r.get(7)?,
                    kind: r.get(8)?,
                    cwd: r.get(9)?,
                    transcript_state: r.get(10)?,
                })
            },
        )
        .optional()?;
    let ex = found.unwrap_or_default();
    let revived = ex.kind.as_deref() == Some("history_only")
        || ex.transcript_state.as_deref() == Some("deleted");
    let reset = reset_in || revived;

    let cwd = if reset {
        d.cwd.clone().or(ex.cwd.clone())
    } else {
        ex.cwd.clone().or(d.cwd.clone())
    };
    if cwd.as_deref().is_some_and(|c| cfg.is_excluded(c)) {
        return Ok(());
    }
    let root = cwd.as_deref().and_then(|c| super::repo_root(c, repo_cache));

    let (mut title, mut tsrc) = if reset {
        (None, None)
    } else {
        (ex.title.clone(), ex.title_source.clone())
    };
    let cand = d
        .custom_title
        .as_ref()
        .map(|t| (4, "custom", t))
        .or(d.ai_title.as_ref().map(|t| (3, "ai", t)))
        .or(d.agent_title.as_ref().map(|t| (2, "agent", t)));
    if let Some((r, src, t)) = cand {
        if r >= tsrc.as_deref().map_or(0, title_rank) {
            title = Some(t.clone());
            tsrc = Some(src.to_string());
        }
    }
    let first_prompt = if reset {
        d.first_prompt.clone()
    } else {
        ex.first_prompt.clone().or(d.first_prompt.clone())
    };
    if title.is_none() {
        if let Some(p) = &first_prompt {
            title = Some(prompt_title(p));
            tsrc = Some("prompt".into());
        }
    }
    let (p_started, p_last, p_prompts, p_replies, p_recap_at) = if reset {
        (None, None, 0, 0, 0)
    } else {
        (
            ex.started_at,
            ex.last_activity_at,
            ex.prompt_count,
            ex.reply_count,
            ex.recap_at.unwrap_or(0),
        )
    };
    let last = [p_last, d.last_ts]
        .into_iter()
        .flatten()
        .max()
        .or(Some(f.stamp.mtime));
    let started = [p_started, d.first_ts].into_iter().flatten().min().or(last);
    let kind = if d.bg || ex.kind.as_deref() == Some("bg") {
        "bg"
    } else {
        "interactive"
    };
    let recap = d.recap.as_ref().filter(|(t, _)| *t >= p_recap_at);

    tx.execute(
        "INSERT INTO session(id, agent) VALUES(?1, ?2) ON CONFLICT(id) DO NOTHING",
        params![sid, agent.id()],
    )?;
    tx.execute(
        "UPDATE session SET project_dir=?2, cwd=?3, repo_root=?4, title=?5, title_source=?6, first_prompt=?7,
            started_at=?8, last_activity_at=?9, kind=?10, transcript_path=?11, transcript_state='present',
            git_branch=COALESCE(?12,git_branch), prompt_count=?13, reply_count=?14,
            last_user_text=COALESCE(?15,last_user_text), last_assistant_text=COALESCE(?16,last_assistant_text),
            recap=COALESCE(?17,recap), recap_at=COALESCE(?18,recap_at)
         WHERE id=?1",
        params![
            sid,
            f.project_dir,
            cwd,
            root,
            title,
            tsrc,
            first_prompt,
            started,
            last,
            kind,
            f.path.to_string_lossy(),
            d.branch,
            p_prompts + d.prompt_count as i64,
            p_replies + d.reply_count as i64,
            d.last_user_text,
            d.last_assistant_text,
            recap.map(|r| agent.clean_recap(&r.1)),
            recap.map(|r| r.0),
        ],
    )?;

    // Token counts: the delta on top of what is stored, from zero after a reset.
    let b: [i64; 4] = if reset {
        [0; 4]
    } else {
        tx.query_row(
            "SELECT tok_in, tok_out, tok_cw, tok_cr FROM session WHERE id=?1",
            [sid],
            |r| {
                let g = |i| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0));
                Ok([g(0)?, g(1)?, g(2)?, g(3)?])
            },
        )?
    };
    tx.execute(
        "UPDATE session SET tok_in=?2, tok_out=?3, tok_cw=?4, tok_cr=?5, tok_last_id=?6 WHERE id=?1",
        params![
            sid,
            b[0] + d.usage.input,
            b[1] + d.usage.output,
            b[2] + d.usage.cache_write,
            b[3] + d.usage.cache_read,
            d.last_msg_id
        ],
    )?;

    if reset {
        tx.execute("DELETE FROM jira_cand WHERE session_id=?1", [sid])?;
        tx.execute("DELETE FROM jira_evidence WHERE session_id=?1", [sid])?;
        tx.execute("DELETE FROM output WHERE session_id=?1", [sid])?;
        tx.execute(
            "DELETE FROM stream WHERE session_id=?1 AND kind='output'",
            [sid],
        )?;
        tx.execute("DELETE FROM meta WHERE key=?1", [format!("folders:{sid}")])?;
    }
    let now = now_ms();
    for (k, ts) in &d.jira {
        tx.execute(
            "INSERT OR IGNORE INTO jira_cand(session_id,key,ts) VALUES(?1,?2,?3)",
            params![sid, k, ts],
        )?;
    }
    for (kind, set) in [
        ("url", &d.jira_url_prefixes),
        ("branch", &d.jira_branch_prefixes),
        ("prompt", &d.jira_prompt_prefixes),
    ] {
        for p in set {
            tx.execute(
                "INSERT OR IGNORE INTO jira_evidence(prefix,kind,session_id) VALUES(?1,?2,?3)",
                params![p, kind, sid],
            )?;
        }
    }
    for (prefix, host) in &d.jira_hosts {
        meta_set(tx, &format!("jira_host:{prefix}"), host)?;
    }
    for o in &d.outputs {
        add_output(
            tx,
            sid,
            o.kind,
            &o.value,
            o.url.as_deref(),
            o.title.as_deref(),
            o.ts,
        )?;
    }
    if !d.folders.is_empty() {
        let mk = format!("folders:{sid}");
        let mut counts: BTreeMap<String, u32> = tx
            .query_row("SELECT value FROM meta WHERE key=?1", [&mk], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for (k, v) in &d.folders {
            *counts.entry(k.clone()).or_insert(0) += v;
        }
        meta_set(tx, &mk, &serde_json::to_string(&counts)?)?;
        let mut top: Vec<(&String, &u32)> = counts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        tx.execute(
            "DELETE FROM output WHERE session_id=?1 AND kind='folder'",
            [sid],
        )?;
        tx.execute(
            "DELETE FROM stream WHERE session_id=?1 AND kind='output' AND key LIKE 'folder:%'",
            [sid],
        )?;
        for (dir, _) in top.into_iter().take(5) {
            add_output(tx, sid, "folder", dir, None, None, d.last_ts.unwrap_or(now))?;
        }
    }
    fts_write(
        tx,
        sid,
        &d.prompts_text,
        &d.replies_text,
        reset,
        PROMPTS_CAP,
        REPLIES_CAP,
    )?;
    Ok(())
}

pub(super) fn add_output(
    tx: &Connection,
    sid: &str,
    kind: &str,
    value: &str,
    url: Option<&str>,
    title: Option<&str>,
    ts: i64,
) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO output(session_id,kind,value,url,first_seen_at) VALUES(?1,?2,?3,?4,?5)",
        params![sid, kind, value, url, ts],
    )?;
    let status = match kind {
        "pr" => "opened",
        "artifact" => "published",
        "commit" => "committed",
        "folder" => "written",
        _ => "mentioned",
    };
    tx.execute(
        "INSERT INTO stream(session_id,kind,key,title,status,detail,link,sort,updated_at)
         VALUES(?1,'output',?2,?3,?4,?5,?6,2,?7) ON CONFLICT(session_id,kind,key) DO NOTHING",
        params![
            sid,
            format!("{kind}:{value}"),
            title.unwrap_or(value),
            status,
            kind,
            url,
            ts
        ],
    )?;
    Ok(())
}
