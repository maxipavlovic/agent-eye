//! The stored results (`loose`, replaced on each scan) and the ignored keys (`loose_ignore`).

use super::{Kind, LooseEnd};
use crate::db::Db;
use anyhow::{bail, Result};
use rusqlite::{params, TransactionBehavior};
use std::collections::HashMap;

/// How long an ignored key is remembered after the thing was last seen is not tracked: a key
/// that nobody ignored for this long is dropped, so the table does not grow for ever.
const IGNORE_KEEP_MS: i64 = 90 * 86_400_000;

/// Replace the stored results with `items`, in one transaction.
pub fn replace(db: &Db, items: &[LooseEnd], now: i64) -> Result<()> {
    let tx = rusqlite::Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM loose", [])?;
    {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO loose(session_id, key, kind, line, detail, since_ms, found_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for i in items {
            st.execute(params![
                i.session_id,
                i.key,
                i.kind.as_str(),
                i.line,
                i.detail,
                i.since_ms,
                now
            ])?;
        }
    }
    tx.execute(
        "DELETE FROM loose_ignore WHERE at < ?1",
        [now - IGNORE_KEEP_MS],
    )?;
    tx.commit()?;
    Ok(())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Option<LooseEnd>> {
    let kind: String = r.get(2)?;
    Ok(Kind::parse(&kind).map(|kind| LooseEnd {
        session_id: r.get(0).unwrap_or_default(),
        key: r.get(1).unwrap_or_default(),
        kind,
        line: r.get(3).unwrap_or_default(),
        detail: r.get(4).unwrap_or_default(),
        since_ms: r.get(5).unwrap_or(0),
    }))
}

const COLS: &str = "l.session_id, l.key, l.kind, l.line, l.detail, l.since_ms";

/// The stored items, newest scan; `session` limits them to one session. Ignored ones are left
/// out unless `with_ignored`.
pub fn list(db: &Db, session: Option<&str>, with_ignored: bool) -> Result<Vec<LooseEnd>> {
    let sql = format!(
        "SELECT {COLS} FROM loose l
         WHERE (?1 IS NULL OR l.session_id = ?1)
           AND (?2 OR l.key NOT IN (SELECT key FROM loose_ignore))
         ORDER BY l.session_id, l.kind, l.since_ms DESC, l.key"
    );
    let mut st = db.conn.prepare(&sql)?;
    let rows = st.query_map(params![session, with_ignored], row)?;
    let mut out = vec![];
    for r in rows {
        out.extend(r?);
    }
    Ok(out)
}

/// One stored item by key, ignored or not.
pub fn get(db: &Db, key: &str) -> Result<Option<LooseEnd>> {
    let sql = format!("SELECT {COLS} FROM loose l WHERE l.key = ?1");
    let mut st = db.conn.prepare(&sql)?;
    let mut rows = st.query_map([key], row)?;
    Ok(rows.next().transpose()?.flatten())
}

/// Items per session, not counting the ignored.
pub fn counts(db: &Db) -> Result<HashMap<String, usize>> {
    let mut st = db.conn.prepare(
        "SELECT session_id, COUNT(*) FROM loose
         WHERE key NOT IN (SELECT key FROM loose_ignore) GROUP BY session_id",
    )?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Stop listing one stored item. Unknown keys are refused: only what a scan found can be ignored.
pub fn ignore(db: &Db, key: &str, now: i64) -> Result<()> {
    if get(db, key)?.is_none() {
        bail!("no such leftover: {key}");
    }
    db.conn.execute(
        "INSERT OR REPLACE INTO loose_ignore(key, at) VALUES(?1, ?2)",
        params![key, now],
    )?;
    Ok(())
}

/// Forget a stored item after it was dealt with.
pub(super) fn forget(db: &Db, key: &str) -> Result<()> {
    db.conn.execute("DELETE FROM loose WHERE key = ?1", [key])?;
    Ok(())
}

/// The leading number of a `what` such as `3 commits in repo`.
fn leading(what: &str) -> usize {
    what.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

/// The ports an item's `what` names: `node vite (pid 4, ports 4321, 4350)`.
pub fn ports_of(item: &LooseEnd) -> Vec<u16> {
    let what = item.what();
    let Some(i) = what.rfind("port") else {
        return vec![];
    };
    let tail = what[i..].trim_end_matches(')');
    tail.split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `Leftovers: 2 servers (ports 4321, 4350), 3 commits not pushed`; None when nothing is.
pub fn summary_line(items: &[LooseEnd]) -> Option<String> {
    summary_text(items).map(|t| format!("Leftovers: {t}"))
}

/// `2 servers (ports 4321, 4350), 3 commits not pushed`: what is left, by kind; None when nothing is.
pub fn summary_text(items: &[LooseEnd]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let of = |k: Kind| items.iter().filter(move |i| i.kind == k);
    let (servers, quiet): (Vec<&LooseEnd>, Vec<&LooseEnd>) =
        of(Kind::Process).partition(|i| !ports_of(i).is_empty());
    let mut parts = vec![];
    if !servers.is_empty() {
        let mut ports: Vec<u16> = servers.iter().flat_map(|i| ports_of(i)).collect();
        ports.sort_unstable();
        ports.dedup();
        parts.push(format!(
            "{} ({} {})",
            count(servers.len(), "server", "servers"),
            if ports.len() == 1 { "port" } else { "ports" },
            ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !quiet.is_empty() {
        parts.push(count(quiet.len(), "process", "processes") + " still running");
    }
    let n = of(Kind::Container).count();
    if n > 0 {
        parts.push(count(n, "container", "containers"));
    }
    let n = of(Kind::Worktree).count();
    if n > 0 {
        parts.push(count(n, "worktree", "worktrees"));
    }
    let n: usize = of(Kind::Unpushed).map(|i| leading(i.what())).sum();
    if n > 0 {
        parts.push(format!("{} not pushed", count(n, "commit", "commits")));
    }
    let n: usize = of(Kind::Uncommitted).map(|i| leading(i.what())).sum();
    if n > 0 {
        parts.push(format!("{} not committed", count(n, "file", "files")));
    }
    Some(parts.join(", "))
}
