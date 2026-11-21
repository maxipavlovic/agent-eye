//! Usage by day from the files to the database: the normal read, the backfill of sessions an older
//! eye already read (from the transcript, its sub-agents, or the archive copy), the rule that keeps
//! a message from being counted twice, and the plan limits Codex rollouts carry.

#[allow(dead_code)]
#[path = "support/fixtures.rs"]
mod fixtures;

use agent_eye::agents::{self, Agents};
use agent_eye::archive;
use agent_eye::config::Config;
use agent_eye::db::Db;
use agent_eye::paths::Paths;
use agent_eye::scan::scan;
use agent_eye::usage::{backfill, day_of, limits};
use agent_eye::worked::summary::local_offset_ms;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const SID: &str = "dddddddd-0000-4000-8000-000000000004";

struct Env {
    _tmp: tempfile::TempDir,
    transcript: PathBuf,
    sub: PathBuf,
    paths: Paths,
    cfg: Config,
    db: Db,
}

/// An assistant line; `id` repeats on every content block of a message.
fn assistant(ts: &str, id: &str, model: &str, input: i64, output: i64, cr: i64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{SID}\",\"message\":{{\"id\":\"{id}\",\"model\":\"{model}\",\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}],\"usage\":{{\"input_tokens\":{input},\"output_tokens\":{output},\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":{cr}}}}}}}\n"
    )
}

fn user(ts: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"timestamp\":\"{ts}\",\"cwd\":\"/tmp/demo\",\"sessionId\":\"{SID}\",\"message\":{{\"content\":\"go\"}}}}\n"
    )
}

/// Two days and two models in the main transcript (the first message on two lines), and a
/// sub-agent that used a third model on the second day.
fn main_transcript() -> String {
    user("2026-11-01T12:00:00.000Z")
        + &assistant(
            "2026-11-01T12:01:00.000Z",
            "m1",
            "claude-opus-5-5",
            100,
            10,
            1000,
        )
        + &assistant(
            "2026-11-01T12:01:01.000Z",
            "m1",
            "claude-opus-5-5",
            100,
            10,
            1000,
        )
        + &assistant(
            "2026-11-02T12:00:00.000Z",
            "m2",
            "claude-sonnet-5-5",
            50,
            5,
            500,
        )
}

fn sub_transcript() -> String {
    assistant(
        "2026-11-02T12:05:00.000Z",
        "s1",
        "claude-haiku-5-5",
        20,
        2,
        200,
    )
    .replace(
        "\"type\":\"assistant\"",
        "\"isSidechain\":true,\"type\":\"assistant\"",
    )
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("claude");
        let proj = claude.join("projects/-tmp-demo");
        std::fs::create_dir_all(proj.join(SID).join("subagents")).unwrap();
        let transcript = proj.join(format!("{SID}.jsonl"));
        std::fs::write(&transcript, main_transcript()).unwrap();
        let sub = proj.join(SID).join("subagents/agent-1.jsonl");
        std::fs::write(&sub, sub_transcript()).unwrap();
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let mut cfg = Config::load_or_create(&paths.config_path()).unwrap();
        cfg.override_agent_dir("claude", claude);
        let db = Db::open(&paths.db_path()).unwrap();
        Env {
            _tmp: tmp,
            transcript,
            sub,
            paths,
            cfg,
            db,
        }
    }

    fn agents(&self) -> Agents {
        agents::enabled(&self.cfg)
    }

    fn scan(&mut self) {
        scan(&mut self.db, &self.cfg, false).unwrap();
    }

    fn backfill(&mut self) -> backfill::Pass {
        let agents = self.agents();
        backfill::run_all(&mut self.db, &self.paths, &agents).unwrap()
    }

    /// (day, model) -> [in, out, cache write, cache read] of the session.
    fn rows(&self) -> BTreeMap<(String, String), [i64; 4]> {
        let mut st = self
            .db
            .conn
            .prepare("SELECT day, model, tok_in, tok_out, tok_cw, tok_cr FROM usage_day WHERE session_id=?1")
            .unwrap();
        st.query_map([SID], |r| {
            Ok((
                (r.get(0)?, r.get(1)?),
                [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// The cursors and token columns, which the backfill must leave alone.
    fn untouched(&self) -> String {
        let mut out = String::new();
        let mut st = self
            .db
            .conn
            .prepare("SELECT path, size, mtime, marker FROM cursor ORDER BY path")
            .unwrap();
        for r in st
            .query_map([], |r| {
                Ok(format!(
                    "{}|{}|{}|{:?}\n",
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?
                ))
            })
            .unwrap()
        {
            out += &r.unwrap();
        }
        out += &format!(
            "{:?}",
            self.db
                .conn
                .query_row(
                    "SELECT tok_in, tok_out, tok_cw, tok_cr, tok_sub, tok_last_id FROM session WHERE id=?1",
                    [SID],
                    |r| Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<String>>(5)?
                    ))
                )
                .unwrap()
        );
        out
    }

    /// Everything counted by day against the session's own token columns plus its sub-agents.
    fn matches_columns(&self) -> bool {
        let by_day: i64 = self
            .db
            .conn
            .query_row(
                "SELECT COALESCE(sum(tok_in+tok_out+tok_cw+tok_cr),0) FROM usage_day WHERE session_id=?1",
                [SID],
                |r| r.get(0),
            )
            .unwrap();
        let cols: i64 = self
            .db
            .conn
            .query_row(
                "SELECT tok_in+tok_out+tok_cw+tok_cr+COALESCE(tok_sub,0) FROM session WHERE id=?1",
                [SID],
                |r| r.get(0),
            )
            .unwrap();
        by_day == cols
    }
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn day(iso_ms: i64) -> String {
    day_of(iso_ms, &local_offset_ms)
}

/// 2026-11-01T12:00:00Z and 2026-11-02T12:00:00Z in Unix ms.
const D1: i64 = 1_793_534_400_000;
const D2: i64 = D1 + 86_400_000;

#[test]
fn a_session_is_counted_by_day_and_model_with_its_sub_agents() {
    let mut e = Env::new();
    e.scan();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (1, 0, 0));
    let rows = e.rows();
    let (d1, d2) = (day(D1), day(D2));
    assert_eq!(rows.len(), 3, "{rows:?}");
    // the message on two lines counts once
    assert_eq!(
        rows[&(d1.clone(), "claude-opus-5-5".into())],
        [100, 10, 0, 1000]
    );
    assert_eq!(
        rows[&(d2.clone(), "claude-sonnet-5-5".into())],
        [50, 5, 0, 500]
    );
    assert_eq!(rows[&(d2, "claude-haiku-5-5".into())], [20, 2, 0, 200]);
    assert!(e.matches_columns());
    assert!(e.db.meta_get(backfill::DONE_KEY).unwrap().is_some());
}

#[test]
fn the_backfill_runs_once_touches_nothing_else_and_is_idempotent() {
    let mut e = Env::new();
    e.scan();
    let before = e.untouched();
    e.backfill();
    let counted = e.rows();
    assert_eq!(e.untouched(), before);
    let again = e.backfill();
    assert_eq!((again.counted, again.unreadable), (0, 0));
    assert_eq!(e.rows(), counted);
    // a scan that finds nothing new changes nothing either
    e.scan();
    assert_eq!(e.rows(), counted);
    assert!(e.matches_columns());
}

#[test]
fn a_read_adds_to_a_counted_session_and_is_left_out_of_one_that_is_not() {
    let mut e = Env::new();
    e.scan();
    // not counted yet: the new line waits for the backfill, which reads the whole file
    append(
        &e.transcript,
        &assistant(
            "2026-11-02T13:00:00.000Z",
            "m3",
            "claude-sonnet-5-5",
            7,
            1,
            0,
        ),
    );
    e.scan();
    assert!(e.rows().is_empty());
    e.backfill();
    assert_eq!(
        e.rows()[&(day(D2), "claude-sonnet-5-5".into())],
        [57, 6, 0, 500]
    );
    // counted: the next line is added, once
    append(
        &e.transcript,
        &assistant(
            "2026-11-02T14:00:00.000Z",
            "m4",
            "claude-sonnet-5-5",
            3,
            1,
            0,
        ),
    );
    e.scan();
    e.scan();
    assert_eq!(
        e.rows()[&(day(D2), "claude-sonnet-5-5".into())],
        [60, 7, 0, 500]
    );
    // and so is a new line of a sub-agent
    append(
        &e.sub,
        &assistant(
            "2026-11-02T14:05:00.000Z",
            "s2",
            "claude-haiku-5-5",
            1,
            1,
            0,
        ),
    );
    e.scan();
    assert_eq!(
        e.rows()[&(day(D2), "claude-haiku-5-5".into())],
        [21, 3, 0, 200]
    );
    assert!(e.matches_columns());
}

#[test]
fn a_rewritten_transcript_is_counted_again_from_the_files() {
    let mut e = Env::new();
    e.scan();
    e.backfill();
    std::fs::write(
        &e.transcript,
        user("2026-11-01T12:00:00.000Z")
            + &assistant("2026-11-01T12:01:00.000Z", "n1", "claude-opus-5-5", 1, 1, 0),
    )
    .unwrap();
    e.scan();
    assert!(e.rows().is_empty(), "stale rows are dropped at once");
    e.backfill();
    let rows = e.rows();
    assert_eq!(rows[&(day(D1), "claude-opus-5-5".into())], [1, 1, 0, 0]);
    assert!(rows.contains_key(&(day(D2), "claude-haiku-5-5".into())));
    assert!(e.matches_columns());
}

#[test]
fn a_session_whose_transcript_is_gone_is_counted_from_the_archive_copy() {
    let mut e = Env::new();
    e.scan();
    let agents = e.agents();
    archive::archive_session(&e.db, &e.paths, &agents, SID).unwrap();
    std::fs::remove_file(&e.transcript).unwrap();
    std::fs::remove_file(&e.sub).unwrap();
    e.scan();
    let pass = e.backfill();
    assert_eq!(pass.counted, 1);
    let rows = e.rows();
    assert_eq!(
        rows[&(day(D1), "claude-opus-5-5".into())],
        [100, 10, 0, 1000]
    );
    // the sub-agent files went with the session; only the main transcript was archived
    assert_eq!(rows.len(), 2);
}

#[test]
fn an_unreadable_session_is_given_up_on_and_not_tried_again() {
    let mut e = Env::new();
    e.scan();
    // the index says there is a transcript, and there is neither it nor an archive copy
    e.db.conn
        .execute(
            "UPDATE session SET transcript_path='/nonexistent/x.jsonl' WHERE id=?1",
            [SID],
        )
        .unwrap();
    let pass = e.backfill();
    assert_eq!((pass.counted, pass.unreadable, pass.left), (0, 1, 0));
    assert_eq!(e.backfill().unreadable, 0);
}

// ---- Codex ----

struct Codex {
    _tmp: tempfile::TempDir,
    paths: Paths,
    cfg: Config,
    db: Db,
}

impl Codex {
    fn new() -> Codex {
        let tmp = tempfile::tempdir().unwrap();
        let codex = tmp.path().join("codex");
        fixtures::copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex-home"),
            &codex,
        );
        let paths = Paths {
            home: tmp.path().join("home"),
        };
        paths.ensure_home().unwrap();
        let cfg = Config::parse(&format!(
            "[agents]\nenabled = [\"codex\"]\n[agents.codex]\ndir = \"{}\"\n",
            codex.display()
        ))
        .unwrap();
        let db = Db::open(&paths.db_path()).unwrap();
        Codex {
            _tmp: tmp,
            paths,
            cfg,
            db,
        }
    }
}

#[test]
fn codex_usage_by_day_adds_up_to_the_session_columns_and_names_the_model() {
    let mut c = Codex::new();
    scan(&mut c.db, &c.cfg, false).unwrap();
    let agents = agents::enabled(&c.cfg);
    let pass = backfill::run_all(&mut c.db, &c.paths, &agents).unwrap();
    assert!(pass.counted >= 3, "{pass:?}");
    let mut checked = 0;
    let mut st = c
        .db
        .conn
        .prepare(
            "SELECT s.id, COALESCE(s.tok_in+s.tok_out+s.tok_cw+s.tok_cr,0)+COALESCE(s.tok_sub,0),
                    COALESCE((SELECT sum(tok_in+tok_out+tok_cw+tok_cr) FROM usage_day WHERE session_id=s.id),0)
             FROM session s WHERE s.agent='codex'",
        )
        .unwrap();
    for r in st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .unwrap()
    {
        let (id, cols, days) = r.unwrap();
        assert_eq!(cols, days, "session {id}");
        checked += i64::from(days > 0);
    }
    assert!(checked >= 2, "{checked} sessions with usage");
    let models: Vec<String> =
        c.db.conn
            .prepare("SELECT DISTINCT model FROM usage_day")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
    assert!(models.iter().any(|m| m == "gpt-5"), "{models:?}");
}

#[test]
fn the_newest_codex_limits_are_kept_with_the_time_they_were_seen() {
    let mut c = Codex::new();
    scan(&mut c.db, &c.cfg, false).unwrap();
    let l = limits::load(&c.db.conn, "codex").unwrap().expect("limits");
    let w = &l.windows[0];
    assert_eq!(w.name, "primary");
    assert_eq!(w.window_minutes, Some(43_200));
    assert_eq!(w.used_percent, 0.0);
    assert_eq!(
        w.resets_at,
        Some(1_796_885_878_000),
        "the newest rollout line wins"
    );
    assert!(l.seen_at > 1_790_000_000_000);
    assert_eq!(l.plan.as_deref(), Some("go"));
}
