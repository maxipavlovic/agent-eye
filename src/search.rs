//! FTS5 search over sessions.

use crate::columns;
use crate::db::Db;
use crate::model::{Column, Session};
use anyhow::Result;
use rusqlite::params;
use std::collections::HashSet;

/// A search result.
pub struct Hit {
    pub session: Session,
    pub column: Column,
    pub snippet: String,
    pub score: f64,
}

/// Each word becomes an FTS5 prefix term; punctuation inside a word (`PROJ-123`) makes it a phrase.
/// Words with no letters or digits are dropped.
pub fn build_query(words: &[String], op: &str) -> Option<String> {
    let terms: Vec<String> = words
        .iter()
        .flat_map(|w| w.split_whitespace())
        .map(|w| {
            w.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(op))
    }
}

const DAY_MS: i64 = 24 * 3600 * 1000;

/// 0 = active within 7 days, 1 = within 30 days, 2 = older.
pub fn freshness(last_ms: i64, now_ms: i64) -> u8 {
    match now_ms - last_ms {
        d if d <= 7 * DAY_MS => 0,
        d if d <= 30 * DAY_MS => 1,
        _ => 2,
    }
}

/// Sort key shared by the picker and `find`: archive last, then the match group (0 title, 1 body,
/// 2 fuzzy), then freshness, status, the group's own relevance `rel` (smaller is better), newest first.
pub type RankKey = (bool, u8, u8, u8, i64, i64);
/// The sort key of a result: archive last, then match group, freshness, status, relevance, age.
pub fn rank_key(col: Column, group: u8, rel: i64, last_ms: i64, now_ms: i64) -> RankKey {
    (
        col == Column::Archive,
        group,
        freshness(last_ms, now_ms),
        crate::pick::status_rank(col),
        rel,
        -last_ms,
    )
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

/// The key that puts a session whose id starts with the query ahead of every other hit.
pub const ID_HIT: RankKey = (false, 0, 0, 0, i64::MIN, 0);

/// Search every session (hidden ones too unless filtered by the caller).
pub fn find(db: &Db, words: &[String], limit: usize, all: bool, now_ms: i64) -> Result<Vec<Hit>> {
    find_without(db, words, limit, all, &HashSet::new(), now_ms)
}

/// Like `find`, leaving out the sessions in `hidden`.
pub fn find_without(
    db: &Db,
    words: &[String],
    limit: usize,
    all: bool,
    hidden: &HashSet<String>,
    now_ms: i64,
) -> Result<Vec<Hit>> {
    let want = limit;
    // Recency and status reorder the FTS hits, so look at more than the caller wants.
    let limit = (limit * 4).max(100) + hidden.len();
    let Some(q_and) = build_query(words, " ") else {
        anyhow::bail!("give at least one search word");
    };
    let mut rows = run(db, &q_and, limit, all)?;
    // A session id typed in full or in part comes first, whatever its text says.
    if let Some(prefix) = id_query(words) {
        let mut st = db.conn.prepare(
            "SELECT id FROM session WHERE id LIKE ?1 || '%' ORDER BY last_activity_at DESC LIMIT 5",
        )?;
        let ids: Vec<String> = st
            .query_map([&prefix], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        rows.retain(|(id, _, _)| !ids.contains(id));
        for id in ids.into_iter().rev() {
            rows.insert(0, (id, "session id".into(), f64::MIN));
        }
    }
    if rows.is_empty() && words.len() > 1 {
        if let Some(q_or) = build_query(words, " OR ") {
            rows = run(db, &q_or, limit, all)?;
        }
    }
    let with_outputs = db.session_ids_with_outputs()?;
    let lc: Vec<String> = words
        .iter()
        .flat_map(|w| w.split_whitespace())
        .map(str::to_lowercase)
        .collect();
    let q_len = lc.join(" ").chars().count() as i64;
    let mut out: Vec<(RankKey, Hit)> = vec![];
    for (pos, (id, snippet, score)) in rows.into_iter().enumerate() {
        if hidden.contains(&id) {
            continue;
        }
        let Some(session) = db.session_by_id(&id)? else {
            continue;
        };
        let column = columns::compute(&columns::Input::from_session(
            &session,
            with_outputs.contains(&id),
            now_ms,
        ));
        let title = crate::pick::session_title(&session).to_lowercase();
        let (group, rel) = if lc.iter().all(|w| title.contains(w.as_str())) {
            (0, (title.chars().count() as i64 - q_len).max(0))
        } else {
            (1, pos as i64)
        };
        let id_hit = id_query(words).is_some_and(|p| id.starts_with(&p));
        let key = if id_hit {
            ID_HIT
        } else {
            rank_key(
                column,
                group,
                rel,
                session.last_activity_at.unwrap_or(0),
                now_ms,
            )
        };
        out.push((
            key,
            Hit {
                session,
                column,
                snippet: snippet.split_whitespace().collect::<Vec<_>>().join(" "),
                score,
            },
        ));
    }
    out.sort_by_key(|(k, _)| *k);
    out.truncate(want);
    Ok(out.into_iter().map(|(_, h)| h).collect())
}

fn run(db: &Db, q: &str, limit: usize, all: bool) -> Result<Vec<(String, String, f64)>> {
    let sql = format!(
        "SELECT s.id, snippet(session_fts, -1, '[', ']', '…', 14), bm25(session_fts, 0.0, 10.0, 3.0, 1.0, 5.0) AS rank
         FROM session_fts JOIN session s ON s.rowid = session_fts.rowid
         WHERE session_fts MATCH ?1 {}
         ORDER BY rank, s.last_activity_at DESC LIMIT ?2",
        if all { "" } else { "AND s.archived = 0" }
    );
    let mut st = db.conn.prepare(&sql)?;
    let rows = st.query_map(params![q, limit as i64], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, f64>(2)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100 * DAY_MS;

    /// A session with `body` as its prompt text, last active `days` ago; `ai` picks the column.
    fn add(db: &Db, id: &str, title: &str, ai: &str, body: &str, days: i64) {
        db.conn
            .execute(
                "INSERT INTO session(id,title,cwd,kind,prompt_count,last_activity_at,ai_status,transcript_state)
                 VALUES(?1,?2,'/tmp','interactive',3,?3,?4,'present')",
                params![id, title, NOW - days * DAY_MS, ai],
            )
            .unwrap();
        crate::db::fts_write(&db.conn, id, body, "", true, 10_000, 10_000).unwrap();
    }

    fn ids(h: Vec<Hit>) -> Vec<String> {
        h.into_iter().map(|h| h.session.id).collect()
    }

    fn find_ids(db: &Db, q: &str, all: bool) -> Vec<String> {
        ids(find(db, &[q.to_string()], 10, all, NOW).unwrap())
    }

    #[test]
    fn id_queries() {
        let w = |s: &str| vec![s.to_string()];
        assert_eq!(id_query(&w("5B8FE153")).as_deref(), Some("5b8fe153"));
        assert_eq!(
            id_query(&w("5b8fe153-f7cf")).as_deref(),
            Some("5b8fe153-f7cf")
        );
        assert_eq!(id_query(&w("cafe")), None); // a word, not an id
        assert_eq!(id_query(&w("5b8")), None);
        assert_eq!(id_query(&["5b8fe153".into(), "x".into()]), None);
    }

    #[test]
    fn find_puts_a_session_id_first() {
        let db = Db::open_memory().unwrap();
        add(
            &db,
            "5b8fe153-0000",
            "Pricing currencies",
            "done",
            "old work",
            40,
        );
        add(
            &db,
            "other",
            "Mentions 5b8fe153 in text",
            "needs_you",
            "see 5b8fe153",
            1,
        );
        assert_eq!(find_ids(&db, "5b8fe153", false)[0], "5b8fe153-0000");
    }

    #[test]
    fn freshness_buckets() {
        assert_eq!(freshness(NOW - 7 * DAY_MS, NOW), 0);
        assert_eq!(freshness(NOW - 8 * DAY_MS, NOW), 1);
        assert_eq!(freshness(NOW - 31 * DAY_MS, NOW), 2);
        assert_eq!(freshness(NOW + 5, NOW), 0);
    }

    #[test]
    fn find_recent_body_hit_beats_older_better_bm25() {
        let db = Db::open_memory().unwrap();
        // "old" has the word many times (better bm25), "new" once but is fresh
        add(&db, "old", "Alpha", "paused", &"zebra ".repeat(20), 40);
        add(&db, "new", "Beta", "paused", "one zebra here", 1);
        assert_eq!(find_ids(&db, "zebra", false), ["new", "old"]);
    }

    #[test]
    fn find_title_hit_before_body_hit_and_status_before_relevance() {
        let db = Db::open_memory().unwrap();
        add(&db, "body", "Alpha", "needs_you", "zebra zebra zebra", 1);
        add(&db, "t_done", "zebra notes", "done", "x", 2);
        add(&db, "t_paused", "zebra notes", "paused", "x", 3);
        // title group first; inside it a paused session beats a done one, newer or not
        assert_eq!(
            find_ids(&db, "zebra", false),
            ["t_paused", "t_done", "body"]
        );
    }

    #[test]
    fn find_two_day_paused_beats_sixty_day_done_with_closer_title() {
        let db = Db::open_memory().unwrap();
        add(&db, "closer", "zebra", "done", "x", 60);
        add(
            &db,
            "fresh",
            "zebra planning for the quarter",
            "paused",
            "x",
            2,
        );
        assert_eq!(find_ids(&db, "zebra", false), ["fresh", "closer"]);
    }

    #[test]
    fn find_archive_last() {
        let db = Db::open_memory().unwrap();
        add(&db, "arch", "zebra", "done", "x", 0);
        db.conn
            .execute("UPDATE session SET archived=1 WHERE id='arch'", [])
            .unwrap();
        add(&db, "old", "zebra", "done", "x", 90);
        assert_eq!(find_ids(&db, "zebra", true), ["old", "arch"]);
        assert_eq!(find_ids(&db, "zebra", false), ["old"]);
    }

    #[test]
    fn find_cuts_to_limit_after_reordering() {
        let db = Db::open_memory().unwrap();
        for n in 0..30 {
            add(
                &db,
                &format!("o{n:02}"),
                "Old",
                "paused",
                &"zebra ".repeat(10),
                60,
            );
        }
        add(&db, "fresh", "Fresh", "paused", "zebra", 1);
        let r = ids(find(&db, &["zebra".to_string()], 3, false, NOW).unwrap());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], "fresh");
    }
    #[test]
    fn query_building() {
        let w = vec![
            "catalog".to_string(),
            "mig\"ration!".to_string(),
            "--".to_string(),
        ];
        assert_eq!(
            build_query(&w, " ").unwrap(),
            "\"catalog\"* \"mig ration\"*"
        );
        assert_eq!(
            build_query(&["PROJ-123".to_string()], " ").unwrap(),
            "\"PROJ 123\"*"
        );
        assert!(build_query(&["!!".to_string()], " ").is_none());
    }
}
