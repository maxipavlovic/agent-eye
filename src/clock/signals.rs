//! What the clock reads from the database: the user's prompts (and which were corrections), and the
//! model's side (tool calls, failed ones, thinking), each with a time. Nothing is judged here.

use crate::config::Config;
use crate::quiet::Quiet;
use anyhow::Result;
use rusqlite::{params, Connection};
use std::collections::HashMap;

/// Prompts this long or shorter, that came after an agent turn, may be corrections. A longer one
/// is a pasted document or a task, not pushback.
pub const MAX_PROMPT_CHARS: i64 = 1500;

pub struct Prompt {
    pub ts: i64,
    pub agent: String,
    /// A model judged it to be pushback on something the agent did wrong.
    pub correction: bool,
    /// A model could still be asked about it and has not been (`UNJUDGED`): it is no correction
    /// yet, but not known to be none either.
    pub pending: bool,
}

/// A tool call, with the agent that made it (agents fail at different rates, so errors are only
/// compared within one agent).
pub struct Call {
    pub ts: i64,
    pub agent: String,
    /// The prompt turn it belongs to (calls and failed calls of one turn share it): failed calls
    /// come in bursts within a turn, so the tests deal with whole turns, never single calls.
    pub turn: i64,
}

pub struct Think {
    pub ts: i64,
    pub agent: String,
    /// The agent's own measure of one thinking block (see `TurnKind::Think`).
    pub n: i64,
}

/// The origins whose sessions the clock leaves out: a script's (`headless`), the night shift's, and
/// those `[board] quiet` keeps quiet. Their prompts are eye's own or a program's, not the user
/// steering. Every query of the clock takes its list from here.
pub fn left_out(cfg: &Config) -> Vec<&'static str> {
    let mut out = vec!["headless", "night"];
    for o in Quiet::of(cfg).origins() {
        if !out.contains(&o) {
            out.push(o);
        }
    }
    out
}

/// `left_out` as an SQL list (`'headless','night'`). The words are eye's own constants, never
/// text from the file.
fn left_out_sql(cfg: &Config) -> String {
    let quoted: Vec<String> = left_out(cfg).iter().map(|o| format!("'{o}'")).collect();
    quoted.join(",")
}

/// The turn events since a time, by kind, oldest first. Sessions of an origin in `left_out` are
/// left out.
#[derive(Default)]
pub struct Data {
    pub prompts: Vec<Prompt>,
    pub interrupts: Vec<i64>,
    pub tools: Vec<Call>,
    pub tool_errors: Vec<Call>,
    pub thinks: Vec<Think>,
}

/// Every event from `from_ms` on.
pub fn load(conn: &Connection, cfg: &Config, from_ms: i64) -> Result<Data> {
    let mut data = Data::default();
    // the turn each session is in: a prompt starts a new one
    let mut open: HashMap<String, i64> = HashMap::new();
    let mut turns = 0;
    let mut st = conn.prepare(
        &format!(
            "SELECT t.ts, t.kind, t.n, s.agent, c.verdict IS 'yes', t.session_id, {}
             FROM turn t JOIN session s ON s.id=t.session_id
             LEFT JOIN correction c ON c.session_id=t.session_id AND c.prompt_ts=t.ts AND t.kind='prompt'
             WHERE t.ts>=?1 AND t.kind IN ('prompt','interrupt','tool','tool_error','think')
               AND COALESCE(s.origin,'') NOT IN ({})
             ORDER BY t.ts",
            unjudged(cfg),
            left_out_sql(cfg),
        ),
    )?;
    let mut rows = st.query(params![from_ms, MAX_PROMPT_CHARS])?;
    while let Some(r) = rows.next()? {
        let mut turn = 0;
        let (ts, kind): (i64, String) = (r.get(0)?, r.get(1)?);
        if matches!(kind.as_str(), "prompt" | "tool" | "tool_error") {
            let session: String = r.get(5)?;
            if kind == "prompt" || !open.contains_key(&session) {
                turns += 1;
                open.insert(session.clone(), turns);
            }
            turn = open[&session];
        }
        match kind.as_str() {
            "prompt" => data.prompts.push(Prompt {
                ts,
                agent: r.get(3)?,
                correction: r.get(4)?,
                pending: r.get(6)?,
            }),
            "interrupt" => data.interrupts.push(ts),
            "tool" => data.tools.push(Call {
                ts,
                agent: r.get(3)?,
                turn,
            }),
            "tool_error" => data.tool_errors.push(Call {
                ts,
                agent: r.get(3)?,
                turn,
            }),
            _ => data.thinks.push(Think {
                ts,
                agent: r.get(3)?,
                n: r.get(2)?,
            }),
        }
    }
    Ok(data)
}

/// The end of every idle span of `min_ms` or longer that ended after `from_ms`: the moment the user
/// came back.
pub fn idle_break_ends(conn: &Connection, from_ms: i64, min_ms: i64) -> Result<Vec<i64>> {
    Ok(conn
        .prepare(
            "SELECT end_ms FROM idle_span WHERE end_ms>=?1 AND end_ms-start_ms>=?2 ORDER BY end_ms",
        )?
        .query_map(params![from_ms, min_ms], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Prompts since `from_ms` that a model could still be asked about and has not been: short enough,
/// after an agent turn, with no verdict stored.
pub fn pending(conn: &Connection, cfg: &Config, from_ms: i64) -> Result<i64> {
    Ok(conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM turn t WHERE {} AND t.ts>=?1",
            unjudged(cfg)
        ),
        params![from_ms, MAX_PROMPT_CHARS],
        |r| r.get(0),
    )?)
}

/// The condition on a prompt turn (`t`) a model could be asked about and has not been (`?2` is the
/// length cap). A session of an origin in `left_out` is not asked about: its prompts are not the
/// user's.
pub fn unjudged(cfg: &Config) -> String {
    format!(
        "t.kind='prompt' AND t.n<=?2
    AND NOT EXISTS(SELECT 1 FROM session s WHERE s.id=t.session_id AND s.origin IN ({}))
    AND EXISTS(SELECT 1 FROM turn e WHERE e.session_id=t.session_id AND e.kind='end' AND e.ts<t.ts)
    AND NOT EXISTS(SELECT 1 FROM correction c WHERE c.session_id=t.session_id AND c.prompt_ts=t.ts)",
        left_out_sql(cfg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    pub(crate) fn db_with(rows: &[(&str, i64, &str, i64)]) -> Db {
        let db = Db::open_memory().unwrap();
        for sid in ["a", "b"] {
            db.conn
                .execute(
                    "INSERT INTO session(id, agent) VALUES(?1, ?2)",
                    params![sid, if sid == "a" { "claude" } else { "codex" }],
                )
                .unwrap();
        }
        for (sid, ts, kind, n) in rows {
            db.conn
                .execute(
                    "INSERT INTO turn(session_id, ts, kind, n) VALUES(?1,?2,?3,?4)",
                    params![sid, ts, kind, n],
                )
                .unwrap();
        }
        db
    }

    #[test]
    fn events_are_read_by_kind_with_their_agent() {
        let db = db_with(&[
            ("a", 10, "prompt", 20),
            ("a", 20, "end", 5),
            ("b", 30, "prompt", 8),
            ("a", 31, "tool", 0),
            ("a", 32, "tool_error", 0),
            ("a", 33, "interrupt", 0),
            ("b", 34, "think", 900),
            ("a", 1, "prompt", 9),
        ]);
        db.conn
            .execute(
                "INSERT INTO correction(session_id, prompt_ts, verdict) VALUES('b', 30, 'yes')",
                [],
            )
            .unwrap();
        let d = load(&db.conn, &Config::default(), 5).unwrap();
        let p: Vec<_> = d
            .prompts
            .iter()
            .map(|p| (p.ts, p.agent.as_str(), p.correction))
            .collect();
        assert_eq!(p, [(10, "claude", false), (30, "codex", true)]);
        assert_eq!(
            (d.tools.len(), d.tool_errors.len(), d.interrupts.len()),
            (1, 1, 1)
        );
        assert_eq!((d.thinks[0].agent.as_str(), d.thinks[0].n), ("codex", 900));
        assert!(load(&db.conn, &Config::default(), 99)
            .unwrap()
            .prompts
            .is_empty());
    }

    #[test]
    fn pending_counts_short_prompts_after_a_turn_without_a_verdict() {
        let db = db_with(&[
            ("a", 10, "prompt", 20),
            ("a", 20, "end", 5),
            ("a", 30, "prompt", 20),
            ("a", 40, "end", 5),
            ("a", 50, "prompt", 20),
            ("a", 60, "end", 5),
            ("a", 70, "prompt", 9000),
        ]);
        // the first prompt follows no turn, the long one is no pushback, one has its verdict
        db.conn
            .execute(
                "INSERT INTO correction(session_id, prompt_ts, verdict) VALUES('a', 30, 'no')",
                [],
            )
            .unwrap();
        assert_eq!(pending(&db.conn, &Config::default(), 0).unwrap(), 1);
        assert_eq!(pending(&db.conn, &Config::default(), 60).unwrap(), 0);
        // the prompts load marks as waiting are exactly the ones counted here
        let marked: Vec<i64> = load(&db.conn, &Config::default(), 0)
            .unwrap()
            .prompts
            .iter()
            .filter(|p| p.pending)
            .map(|p| p.ts)
            .collect();
        assert_eq!(marked, [50]);
    }

    #[test]
    fn scripted_and_night_shift_sessions_are_not_your_steering() {
        let db = db_with(&[
            ("a", 10, "end", 5),
            ("a", 20, "prompt", 20),
            ("b", 30, "end", 5),
            ("b", 40, "prompt", 20),
        ]);
        for origin in ["headless", "night"] {
            db.conn
                .execute("UPDATE session SET origin=?1 WHERE id='b'", [origin])
                .unwrap();
            let d = load(&db.conn, &Config::default(), 0).unwrap();
            assert_eq!(d.prompts.iter().map(|p| p.ts).collect::<Vec<_>>(), [20]);
            // and a model is not asked about them
            assert_eq!(
                pending(&db.conn, &Config::default(), 0).unwrap(),
                1,
                "{origin}"
            );
        }
        // a session of unknown origin, or one started in a terminal, counts
        for origin in [None, Some("terminal")] {
            db.conn
                .execute("UPDATE session SET origin=?1 WHERE id='b'", [origin])
                .unwrap();
            assert_eq!(
                load(&db.conn, &Config::default(), 0).unwrap().prompts.len(),
                2
            );
            assert_eq!(pending(&db.conn, &Config::default(), 0).unwrap(), 2);
        }
    }

    #[test]
    fn sessions_kept_quiet_are_left_out_like_a_script_s() {
        let db = db_with(&[
            ("a", 10, "end", 5),
            ("a", 20, "prompt", 20),
            ("b", 30, "end", 5),
            ("b", 40, "prompt", 20),
            ("b", 41, "tool", 0),
            ("b", 42, "tool_error", 0),
            ("b", 43, "think", 50),
            ("b", 44, "interrupt", 0),
        ]);
        db.conn
            .execute("UPDATE session SET origin='app' WHERE id='b'", [])
            .unwrap();
        let loud = Config::default();
        let d = load(&db.conn, &loud, 0).unwrap();
        assert_eq!((d.prompts.len(), d.tools.len(), d.thinks.len()), (2, 1, 1));
        assert_eq!(pending(&db.conn, &loud, 0).unwrap(), 2);
        let mut quiet = Config::default();
        quiet.board.quiet = vec!["app".into()];
        let d = load(&db.conn, &quiet, 0).unwrap();
        assert_eq!(d.prompts.iter().map(|p| p.ts).collect::<Vec<_>>(), [20]);
        assert!(d.tools.is_empty() && d.tool_errors.is_empty());
        assert!(d.thinks.is_empty() && d.interrupts.is_empty());
        // and a model is not asked about them
        assert_eq!(pending(&db.conn, &quiet, 0).unwrap(), 1);
        // one list: a script's and the night shift's always, a quiet origin once
        assert_eq!(left_out(&loud), ["headless", "night"]);
        assert_eq!(left_out(&quiet), ["headless", "night", "app"]);
        quiet.board.quiet = vec!["headless".into(), "app".into()];
        assert_eq!(left_out(&quiet), ["headless", "night", "app"]);
    }

    #[test]
    fn idle_breaks_are_the_spans_that_are_long_enough() {
        let db = Db::open_memory().unwrap();
        for (s, e) in [(0, 1_000), (5_000, 9_000), (20_000, 30_000)] {
            db.conn
                .execute(
                    "INSERT INTO idle_span(start_ms, end_ms) VALUES(?1,?2)",
                    [s, e],
                )
                .unwrap();
        }
        assert_eq!(
            idle_break_ends(&db.conn, 0, 3_000).unwrap(),
            [9_000, 30_000]
        );
        assert_eq!(idle_break_ends(&db.conn, 10_000, 3_000).unwrap(), [30_000]);
    }
}
