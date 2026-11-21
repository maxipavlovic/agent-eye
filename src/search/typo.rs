//! Fixing a mistyped word from the words the index already holds.

use crate::db::Db;

/// The most alternatives tried for one word.
const MAX_ALTERNATIVES: usize = 3;

/// Edit distance counting a swap of two neighbours as one change (optimal string alignment).
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

/// Up to three indexed words close to `word` (lowercase): the same first letter, one change away
/// (two when the word has 7+ letters), nearest first, then the most common. The index holds word
/// stems, so a stem is a fine answer. Only the words that can qualify are read.
pub fn alternatives(db: &Db, word: &str) -> Vec<String> {
    let len = word.chars().count();
    let limit = if len >= 7 { 2 } else { 1 };
    let Some(first) = word.chars().next() else {
        return vec![];
    };
    let run = || -> rusqlite::Result<Vec<(usize, i64, String)>> {
        let mut st = db.conn.prepare_cached(
            "SELECT term, doc FROM session_fts_vocab
             WHERE term >= ?1 AND term < ?2 AND length(term) BETWEEN ?3 AND ?4",
        )?;
        let rows = st.query_map(
            rusqlite::params![
                first.to_string(),
                format!("{first}\u{10FFFF}"),
                len.saturating_sub(limit) as i64,
                (len + limit) as i64
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )?;
        let mut found = vec![];
        for row in rows {
            let (term, doc) = row?;
            let d = distance(word, &term);
            if d > 0 && d <= limit {
                found.push((d, doc, term));
            }
        }
        Ok(found)
    };
    let mut found = run().unwrap_or_default();
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    found
        .into_iter()
        .take(MAX_ALTERNATIVES)
        .map(|(_, _, t)| t)
        .collect()
}
