use super::*;
use crate::agents::claude::Claude;
use crate::clock::verdict::Kind;
use crate::db::Db;
use std::sync::Arc;

const H: i64 = 3_600_000;
const DAY: i64 = 86_400_000;
/// 2026-11-10 00:00 UTC.
const D0: i64 = 20_767 * DAY;
/// Three weeks on, at 19:30, so a four-week window holds all of the days seeded.
const NOW: i64 = D0 + 21 * DAY + 19 * H + 30 * 60_000;

fn utc(_: i64) -> i64 {
    0
}

struct World {
    db: Db,
    n: i64,
}

impl World {
    fn new() -> World {
        let db = Db::open_memory().unwrap();
        db.conn
            .execute("INSERT INTO session(id, agent) VALUES('s','claude')", [])
            .unwrap();
        World { db, n: 0 }
    }

    fn turn(&mut self, ts: i64, kind: &str, n: i64) {
        self.db
            .conn
            .execute(
                "INSERT INTO turn(session_id, ts, kind, n) VALUES('s',?1,?2,?3)",
                rusqlite::params![ts, kind, n],
            )
            .unwrap();
    }

    /// `prompts` prompts in local hour `h`, of which `corrections` were corrections, each with an
    /// agent turn before it, 20 tool calls (2 failed) and one thought of `think`.
    fn hour(&mut self, h: i64, prompts: i64, corrections: i64, think: i64) {
        for i in 0..prompts {
            self.n += 1;
            let ts = D0 + (self.n % 20) * DAY + h * H + i * 1000;
            self.turn(ts - 500, "end", 5);
            self.turn(ts, "prompt", 30);
            if i < corrections {
                self.db
                    .conn
                    .execute(
                        "INSERT INTO correction(session_id, prompt_ts, verdict, model) VALUES('s',?1,'yes','haiku')",
                        [ts],
                    )
                    .unwrap();
            }
            for k in 0..20 {
                self.turn(ts + 10 + k, if k < 2 { "tool_error" } else { "tool" }, 0);
            }
            self.turn(ts + 40, "think", think);
        }
    }

    /// Nine day hours at 8% and two evening hours at 16%.
    fn steady_days_worse_evenings(&mut self, evening_think: i64) {
        for h in 9..18 {
            self.hour(h, 50, 4, 1000);
        }
        for h in 19..21 {
            self.hour(h, 100, 16, evening_think);
        }
    }

    fn report(&self, cfg: &str) -> Report {
        let agents = Agents::from_vec(vec![Arc::new(Claude::new("/nonexistent".into()))]);
        build(
            &self.db.conn,
            &Config::parse(cfg).unwrap(),
            &agents,
            NOW,
            4,
            &utc,
        )
        .unwrap()
    }
}

#[test]
fn worse_evenings_with_a_steady_model_are_you() {
    let mut w = World::new();
    w.steady_days_worse_evenings(1000);
    let r = w.report("");
    assert_eq!((r.prompts, r.corrections), (650, 68));
    assert_eq!(
        r.bad,
        [Range {
            from_h: 19,
            to_h: 21
        }]
    );
    assert_eq!(r.verdict.kind, Kind::You);
    assert_eq!(
        (r.verdict.you_ratio, r.verdict.you_n),
        (Some(2.0), Some(32))
    );
    assert_eq!(r.verdict.think_ratio, Some(1.0));
    assert!(r
        .verdict
        .text
        .starts_with("Evenings: 2.0x more corrections (32)."));
    assert_eq!(
        (r.hours.len(), r.hours[19].prompts, r.hours[19].think),
        (24, 100, Some(1000))
    );
    assert_eq!(
        (r.from.as_str(), r.to.as_str()),
        ("2026-11-03", "2026-12-01")
    );
}

#[test]
fn a_model_that_thinks_less_in_those_hours_makes_it_both() {
    let mut w = World::new();
    w.steady_days_worse_evenings(700);
    let r = w.report("");
    assert_eq!(r.verdict.kind, Kind::Both);
    assert_eq!(r.verdict.think_ratio, Some(0.7));
    assert!(r.verdict.text.ends_with("Both moved."));
}

#[test]
fn a_model_that_slips_while_you_stay_steady_is_the_model() {
    let mut w = World::new();
    for h in 9..18 {
        w.hour(h, 50, 4, 1000);
    }
    // an hour with a high rate but too few corrections to rule out chance, and a shallower model
    w.hour(19, 20, 3, 500);
    let r = w.report("");
    assert_eq!(r.verdict.kind, Kind::Model);
    assert!(r.bad.is_empty());
    assert_eq!(
        r.verdict.text,
        "Evenings: your corrections about the same. Model thinking \u{2212}50%. Tool errors flat. Mostly the model."
    );
}

#[test]
fn no_hour_standing_out_is_no_pattern() {
    let mut w = World::new();
    for h in 9..18 {
        w.hour(h, 50, 4, 1000);
    }
    let r = w.report("");
    assert_eq!(r.verdict.kind, Kind::None);
    assert!(r.bad.is_empty());
}

#[test]
fn too_few_prompts_or_corrections_say_so() {
    let mut w = World::new();
    w.hour(10, 100, 5, 1000);
    let r = w.report("");
    assert_eq!(r.verdict.kind, Kind::TooFew);
    assert!(r
        .verdict
        .text
        .starts_with("Too few to tell yet: 100 prompts and 5 corrections"));
    assert!(r.bad.is_empty());
}

#[test]
fn an_empty_database_gives_a_whole_empty_report() {
    let r = World::new().report("");
    assert_eq!((r.prompts, r.corrections, r.pending), (0, 0, 0));
    assert_eq!(r.hours.len(), 24);
    assert_eq!(r.since_break.len(), 4);
    assert_eq!(r.verdict.kind, Kind::TooFew);
    assert_eq!(
        (r.now.in_bad, r.now.since_break_min, r.now.hint.clone()),
        (false, None, None)
    );
    assert!(r.agents.is_empty());
}

#[test]
fn a_switched_off_clock_reports_nothing() {
    let mut w = World::new();
    w.steady_days_worse_evenings(1000);
    let r = w.report("[clock]\nenabled = false");
    assert!(!r.enabled && !r.llm);
    assert_eq!(r.prompts, 0);
    assert_eq!(r.verdict.kind, Kind::TooFew);
}

#[test]
fn with_judging_off_nothing_is_pending_and_nothing_is_corrected() {
    let mut w = World::new();
    w.steady_days_worse_evenings(1000);
    let on = w.report("");
    assert!(on.llm && on.pending > 0);
    let off = w.report("[clock]\nllm = false");
    assert!(!off.llm);
    assert_eq!(off.pending, 0);
}

#[test]
fn the_hint_is_off_by_default_and_quiet_outside_the_bad_hours() {
    let mut w = World::new();
    w.steady_days_worse_evenings(1000);
    // now is 19:30, inside the bad band
    let off = w.report("");
    assert!(off.now.in_bad);
    assert_eq!(off.now.hint, None);
    let on = w.report("[clock]\nhints = true");
    assert_eq!(
        on.now.hint.as_deref(),
        Some("Evening: your corrections usually run 2.0x now. Hand this to the night shift? (T)")
    );
    // the same data seen at 11:00 is outside it
    let agents = Agents::default();
    let cfg = Config::parse("[clock]\nhints = true").unwrap();
    let morning = build(
        &w.db.conn,
        &cfg,
        &agents,
        NOW - 8 * H - 30 * 60_000,
        4,
        &utc,
    )
    .unwrap();
    assert!(!morning.now.in_bad && morning.now.hint.is_none());
}

#[test]
fn since_break_is_zero_after_a_long_quiet_and_counts_from_the_last_break_otherwise() {
    let mut w = World::new();
    for (m, correction) in [(0, false), (10, false), (20, true)] {
        w.turn(NOW - 30 * 60_000 + m * 60_000, "prompt", 10);
        if correction {
            w.db.conn
                .execute(
                    "INSERT INTO correction(session_id, prompt_ts, verdict) VALUES('s',?1,'yes')",
                    [NOW - 30 * 60_000 + m * 60_000],
                )
                .unwrap();
        }
    }
    let r = w.report("");
    // the stretch began 30 minutes ago, the last prompt 10 minutes ago
    assert_eq!(r.now.since_break_min, Some(30));
    assert_eq!(r.since_break[0].prompts, 3);
    let later = build(
        &w.db.conn,
        &Config::default(),
        &Agents::default(),
        NOW + 2 * H,
        4,
        &utc,
    )
    .unwrap();
    assert_eq!(later.now.since_break_min, Some(0));
}

#[test]
fn breaks_come_from_idle_spans_when_idle_is_on() {
    let mut w = World::new();
    for m in [0, 5, 70, 75] {
        w.turn(NOW - 100 * 60_000 + m * 60_000, "prompt", 10);
    }
    // the user was away from minute 20 to minute 60 of that stretch
    w.db.conn
        .execute(
            "INSERT INTO idle_span(start_ms, end_ms) VALUES(?1,?2)",
            [NOW - 80 * 60_000, NOW - 40 * 60_000],
        )
        .unwrap();
    let by_gaps = w.report("[clock]\nbreak_minutes = 30");
    let by_idle = w.report("[clock]\nbreak_minutes = 30\nidle = true");
    assert!(by_idle.idle && !by_gaps.idle);
    let counts = |r: &Report| r.since_break.iter().map(|b| b.prompts).collect::<Vec<_>>();
    // by gaps: breaks at minute 0 and 70; the second stretch has two prompts at 0 and 5 minutes
    assert_eq!(counts(&by_gaps), [4, 0, 0, 0]);
    // by idle: only prompts after the span count: 30 and 35 minutes after it ended
    assert_eq!(counts(&by_idle), [2, 0, 0, 0]);
}

#[test]
fn agents_are_listed_with_their_thinking_unit() {
    let mut w = World::new();
    w.hour(10, 3, 1, 100);
    let r = w.report("");
    assert_eq!(
        r.agents,
        [AgentRow {
            agent: "claude".into(),
            prompts: 3,
            corrections: 1,
            think: Some("signature")
        }]
    );
}

#[test]
fn the_json_has_the_documented_fields() {
    let mut w = World::new();
    w.steady_days_worse_evenings(1000);
    let j = w.report("").json();
    let keys = |v: &serde_json::Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(
        keys(&j),
        [
            "agents",
            "bad",
            "corrections",
            "enabled",
            "from",
            "hints",
            "hours",
            "idle",
            "llm",
            "now",
            "pending",
            "prompts",
            "since_break",
            "to",
            "verdict"
        ]
    );
    assert_eq!(
        keys(&j["hours"][0]),
        [
            "corrections",
            "h",
            "interrupts",
            "prompts",
            "think",
            "think_n",
            "tool_calls",
            "tool_errors"
        ]
    );
    assert_eq!(
        keys(&j["verdict"]),
        [
            "kind",
            "text",
            "think_ratio",
            "tool_error_ratio",
            "you_n",
            "you_ratio"
        ]
    );
    assert_eq!(keys(&j["now"]), ["hint", "in_bad", "since_break_min"]);
    assert_eq!(j["verdict"]["kind"], "you");
    assert_eq!(j["bad"][0], serde_json::json!({"from_h": 19, "to_h": 21}));
    assert_eq!(
        keys(&j["since_break"][0]),
        ["band", "corrections", "prompts"]
    );
}
