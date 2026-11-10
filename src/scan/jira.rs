//! Jira keys: which of the project-shaped keys seen in transcripts are real projects, and the
//! outputs that follow.

use super::transcript::add_output;
use crate::config::Config;
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Project prefixes that count as real Jira projects: the config override when given, else
/// those seen in `atlassian.net/browse/` URLs or git branch names, or typed in the user's own
/// prompts in at least two sessions.
pub fn known_jira_projects(conn: &Connection, cfg: &Config) -> Result<BTreeSet<String>> {
    if !cfg.outputs.jira_projects.is_empty() {
        return Ok(cfg
            .outputs
            .jira_projects
            .iter()
            .map(|p| p.trim().to_uppercase())
            .filter(|p| !p.is_empty())
            .collect());
    }
    let mut st = conn.prepare(
        "SELECT prefix FROM jira_evidence GROUP BY prefix
         HAVING SUM(kind IN ('url','branch')) > 0
             OR COUNT(DISTINCT CASE WHEN kind='prompt' THEN session_id END) >= 2",
    )?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Make the `jira` outputs match the candidates whose project prefix is known.
pub(super) fn refresh(tx: &Connection, cfg: &Config) -> Result<()> {
    let known = known_jira_projects(tx, cfg)?;
    let mut want: BTreeMap<(String, String), i64> = BTreeMap::new();
    {
        let mut st = tx.prepare("SELECT session_id, key, ts FROM jira_cand")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (sid, key, ts) = r?;
            let prefix = key.split('-').next().unwrap_or("");
            if known.contains(prefix) {
                want.insert((sid, key), ts);
            }
        }
    }
    let mut have: HashSet<(String, String)> = HashSet::new();
    {
        let mut st = tx.prepare("SELECT session_id, value FROM output WHERE kind='jira'")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for r in rows {
            have.insert(r?);
        }
    }
    for (sid, key) in have.iter().filter(|k| !want.contains_key(*k)) {
        tx.execute(
            "DELETE FROM output WHERE session_id=?1 AND kind='jira' AND value=?2",
            params![sid, key],
        )?;
        tx.execute(
            "DELETE FROM stream WHERE session_id=?1 AND kind='output' AND key=?2",
            params![sid, format!("jira:{key}")],
        )?;
    }
    for ((sid, key), ts) in want.iter().filter(|(k, _)| !have.contains(*k)) {
        let exists: bool = tx
            .query_row("SELECT 1 FROM session WHERE id=?1", [sid], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists {
            add_output(tx, sid, "jira", key, None, None, *ts)?;
        }
    }
    Ok(())
}
