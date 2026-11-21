//! Search over sessions, one engine for the picker, `eye find` and the board.
//!
//! The index is SQLite FTS5 with word stemming. `query` reads what was typed, `typo` fixes a word
//! that matches nothing, `score` weighs every unit by how rare it is and where it is found. This
//! file holds the order of the results and `find`.

pub mod query;
pub mod score;
mod typo;

use crate::columns;
use crate::db::Db;
use crate::model::{Column, Session};
use anyhow::Result;
use rusqlite::OptionalExtension;
use std::cmp::Ordering;
use std::collections::HashSet;

pub use score::{score_words, Matched, Scored};

/// A search result.
pub struct Hit {
    pub session: Session,
    pub column: Column,
    pub snippet: String,
    /// How well it matches; higher is better.
    pub score: f64,
}

/// Is there anything to search for in these words (not empty, not only punctuation)?
pub fn has_terms(words: &[String]) -> bool {
    !query::parse(words).is_empty()
}

/// Where a result stands. Best first: a session id typed in part, then the scored matches (by score,
/// then the fewest title words beyond what was typed, then the newest), then the fuzzy title
/// matches. Sessions the user archived come after everything else. Status and age never lift one.
#[derive(Debug, Clone, Copy)]
pub struct Rank {
    pub id_hit: bool,
    pub archived: bool,
    /// 0 = scored match, 1 = fuzzy title match.
    pub group: u8,
    pub score: f64,
    /// Title words beyond the number of words typed.
    pub extra: usize,
    /// Last activity, Unix ms.
    pub last: i64,
}

impl Rank {
    /// Put the better result first.
    pub fn order(&self, o: &Rank) -> Ordering {
        o.id_hit
            .cmp(&self.id_hit)
            .then(self.archived.cmp(&o.archived))
            .then(self.group.cmp(&o.group))
            .then(o.score.total_cmp(&self.score))
            .then(self.extra.cmp(&o.extra))
            .then(o.last.cmp(&self.last))
    }
}

/// How many words a title has beyond the `typed` ones (0 when it has fewer).
pub fn extra_words(title: &str, typed: usize) -> usize {
    title
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count()
        .saturating_sub(typed)
}

/// A query that looks like the start of a session id ("5b8fe153", a full uuid): 4+ hex digits and dashes.
pub fn id_query(words: &[String]) -> Option<String> {
    let [w] = words else { return None };
    let w = w.trim().to_lowercase();
    (w.len() >= 4
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
    .then_some(w)
}

/// Search every session (hidden ones too unless filtered by the caller).
pub fn find(db: &Db, words: &[String], limit: usize, all: bool, now_ms: i64) -> Result<Vec<Hit>> {
    find_without(db, words, limit, all, &HashSet::new(), now_ms)
}

/// What the order of a session needs besides its score.
struct Facts {
    title: String,
    last: i64,
    archived: bool,
}

fn facts(db: &Db, id: &str) -> Result<Option<Facts>> {
    let mut st = db.conn.prepare_cached(
        "SELECT title, first_prompt, last_activity_at, archived FROM session WHERE id=?1",
    )?;
    Ok(st
        .query_row([id], |r| {
            let title: Option<String> = r.get(0)?;
            let first: Option<String> = r.get(1)?;
            Ok(Facts {
                title: crate::pick::title_line(title.as_deref(), first.as_deref()).to_lowercase(),
                last: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                archived: r.get::<_, i64>(3)? != 0,
            })
        })
        .optional()?)
}

/// Like `find`, leaving out the sessions in `hidden`. Archived sessions are listed only with
/// `all`, and then last.
pub fn find_without(
    db: &Db,
    words: &[String],
    limit: usize,
    all: bool,
    hidden: &HashSet<String>,
    now_ms: i64,
) -> Result<Vec<Hit>> {
    let q = query::parse(words);
    if q.is_empty() {
        return Ok(vec![]);
    }
    let scored = score::score(db, &q)?;
    // A session id typed in full or in part comes first, whatever its text says.
    let mut id_hits: Vec<String> = vec![];
    if let Some(prefix) = id_query(words) {
        let mut st = db.conn.prepare(
            "SELECT id FROM session WHERE id LIKE ?1 || '%' ORDER BY last_activity_at DESC LIMIT 5",
        )?;
        id_hits = st
            .query_map([&prefix], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    let mut ranked: Vec<(Rank, String)> = vec![];
    let candidates = scored
        .by_id
        .iter()
        .map(|(id, m)| (id, m.score))
        .chain(id_hits.iter().map(|id| (id, 0.0)));
    let mut seen = HashSet::new();
    for (id, score) in candidates {
        if hidden.contains(id) || !seen.insert(id.as_str()) {
            continue;
        }
        let Some(f) = facts(db, id)? else { continue };
        if f.archived && !all {
            continue;
        }
        let id_hit = id_hits.contains(id);
        let score = scored.by_id.get(id).map_or(score, |m| m.score);
        ranked.push((
            Rank {
                id_hit,
                archived: f.archived,
                group: 0,
                score,
                extra: extra_words(&f.title, scored.words),
                last: f.last,
            },
            id.clone(),
        ));
    }
    ranked.sort_by(|a, b| a.0.order(&b.0));
    ranked.truncate(limit);
    let with_outputs = db.session_ids_with_outputs()?;
    let or_query = scored.exprs.join(" OR ");
    let mut out = vec![];
    for (rank, id) in ranked {
        let Some(session) = db.session_by_id(&id)? else {
            continue;
        };
        let column = columns::compute(&columns::Input::from_session(
            &session,
            with_outputs.contains(&id),
            now_ms,
        ));
        let snippet = if rank.id_hit {
            "session id".to_string()
        } else {
            let rowid = scored.by_id.get(&id).map_or(0, |m| m.rowid);
            snippet(db, &or_query, rowid)?
        };
        out.push(Hit {
            session,
            column,
            snippet,
            score: if rank.id_hit { f64::MAX } else { rank.score },
        });
    }
    Ok(out)
}

/// The best passage of a session for the words, with the matches in `[brackets]`.
fn snippet(db: &Db, or_query: &str, rowid: i64) -> Result<String> {
    let mut st = db.conn.prepare_cached(
        "SELECT snippet(session_fts, -1, '[', ']', '…', 14) FROM session_fts
         WHERE session_fts MATCH ?1 AND rowid = ?2",
    )?;
    let s: Option<String> = st
        .query_row(rusqlite::params![or_query, rowid], |r| r.get(0))
        .optional()?;
    Ok(s.unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" "))
}

#[cfg(test)]
mod tests;
