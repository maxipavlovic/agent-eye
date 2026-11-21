//! Scoring: which sessions match a query, and how well.
//!
//! Every unit of the query gets a weight from how rare it is (`idf`, from the number of sessions
//! that hold it), times the weight of the best field it is found in. A session's score is the sum
//! over the units; it must match at least half of them.

use super::query::{self, Query};
use super::typo;
use crate::db::Db;
use anyhow::Result;
use std::collections::HashMap;

/// The fields of the index and what a match in each is worth.
const FIELDS: [(&str, f64); 4] = [
    ("title", 3.0),
    ("extra", 2.0),
    ("prompts", 1.5),
    ("replies", 1.0),
];
/// What a match in an idea's title, and in its notes, is worth.
const IDEA_TITLE: f64 = 3.0;
const IDEA_NOTES: f64 = 2.0;

/// A session that matched.
#[derive(Debug, Clone)]
pub struct Matched {
    /// Its row in the search index (the same number as the session's rowid).
    pub rowid: i64,
    pub score: f64,
}

/// What a query found, with what is needed to score an idea the same way.
#[derive(Debug, Default)]
pub struct Scored {
    /// Matching sessions by id.
    pub by_id: HashMap<String, Matched>,
    /// Per unit: its weight and the text an idea must contain.
    units: Vec<(f64, String)>,
    /// How many units a row must match.
    need: usize,
    /// The words typed.
    pub words: usize,
    /// The FTS5 expression of every unit, typo fixes included; for snippets.
    pub exprs: Vec<String>,
}

impl Scored {
    /// The score of an idea (lowercase title and notes), found by substring as an idea is not in
    /// the index; `None` when it holds fewer than half of the units.
    pub fn idea(&self, title_lc: &str, notes_lc: &str) -> Option<f64> {
        let (mut total, mut hit) = (0.0, 0);
        for (idf, needle) in &self.units {
            let w = if title_lc.contains(needle.as_str()) {
                IDEA_TITLE
            } else if notes_lc.contains(needle.as_str()) {
                IDEA_NOTES
            } else {
                continue;
            };
            total += idf * w;
            hit += 1;
        }
        (hit > 0 && hit >= self.need).then_some(total)
    }

    /// True when the query held nothing to search for.
    pub fn is_blank(&self) -> bool {
        self.units.is_empty()
    }
}

/// The rowids that match `expr` in `field`.
fn rows_in(db: &Db, field: &str, expr: &str) -> rusqlite::Result<Vec<i64>> {
    let mut st = db
        .conn
        .prepare_cached("SELECT rowid FROM session_fts WHERE session_fts MATCH ?1")?;
    let rows = st.query_map([format!("{field} : ({expr})")], |r| r.get::<_, i64>(0))?;
    rows.collect()
}

/// For every row that matches `expr`, the weight of the best field it matches in.
fn best_fields(db: &Db, expr: &str) -> rusqlite::Result<HashMap<i64, f64>> {
    let mut best: HashMap<i64, f64> = HashMap::new();
    for (field, weight) in FIELDS {
        for rowid in rows_in(db, field, expr)? {
            let w = best.entry(rowid).or_insert(0.0);
            *w = w.max(weight);
        }
    }
    Ok(best)
}

/// The weight of a unit that `df` of `n` rows hold.
fn idf(n: i64, df: usize) -> f64 {
    let (n, df) = (n as f64, df as f64);
    ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
}

/// Score every session against a query.
pub fn score(db: &Db, q: &Query) -> Result<Scored> {
    let mut out = Scored {
        need: q.need(),
        words: q.words,
        ..Scored::default()
    };
    if q.is_empty() {
        return Ok(out);
    }
    let n: i64 = db
        .conn
        .prepare_cached("SELECT count(*) FROM session_fts")?
        .query_row([], |r| r.get(0))?;
    let mut totals: HashMap<i64, (f64, usize)> = HashMap::new();
    for unit in &q.units {
        let mut expr = unit.phrase();
        let mut best = best_fields(db, &expr)?;
        if best.is_empty() && unit.typo_ok {
            let alts = typo::alternatives(db, &unit.parts[0]);
            if !alts.is_empty() {
                // fixed words match whole
                expr = alts
                    .iter()
                    .map(|a| format!("\"{a}\""))
                    .collect::<Vec<_>>()
                    .join(" OR ");
                best = best_fields(db, &expr)?;
            }
        }
        let idf = idf(n, best.len());
        for (rowid, w) in best {
            let t = totals.entry(rowid).or_default();
            t.0 += idf * w;
            t.1 += 1;
        }
        out.units.push((idf, unit.needle.clone()));
        out.exprs.push(expr);
    }
    let mut st = db
        .conn
        .prepare_cached("SELECT session_id FROM session_fts WHERE rowid=?1")?;
    for (rowid, (score, units)) in totals {
        if units >= out.need {
            let id: String = st.query_row([rowid], |r| r.get(0))?;
            out.by_id.insert(id, Matched { rowid, score });
        }
    }
    Ok(out)
}

/// Score the words of a search.
pub fn score_words(db: &Db, words: &[String]) -> Result<Scored> {
    score(db, &query::parse(words))
}
