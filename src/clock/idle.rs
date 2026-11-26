//! Idle spans: stretches of 5 minutes or more with no keyboard or mouse input, kept so a break is
//! the time the user was really away and not only a gap between prompts. The daemon samples the
//! system's idle time once a minute while `[clock] idle` is on (`platform::idle`); this turns the
//! samples into spans. A span is stored with its start and end, nothing else.

use crate::agents::Check;
use crate::config::Config;
use crate::platform::idle::{idle_ms, Sys};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// The shortest stretch that is stored.
pub const MIN_SPAN_MS: i64 = 5 * 60_000;
/// A span found again after a restart may start this far from where it started before.
const SAME_START_MS: i64 = 5_000;

/// Turns idle-time samples into stored spans.
#[derive(Default)]
pub struct Sampler {
    /// The start of the span that is going on.
    open: Option<i64>,
}

impl Sampler {
    /// One sample: the machine has been idle for `idle_ms` at `now`. Idle past the minimum opens (or
    /// extends) a span; the first sample after it closes the span at the moment the input came back.
    pub fn sample(&mut self, conn: &Connection, now: i64, idle_ms: i64) -> Result<()> {
        if idle_ms >= MIN_SPAN_MS {
            let start = match self.open {
                Some(s) => s,
                None => same_span(conn, now - idle_ms)?.unwrap_or(now - idle_ms),
            };
            conn.execute(
                "INSERT INTO idle_span(start_ms, end_ms) VALUES(?1,?2)
                 ON CONFLICT(start_ms) DO UPDATE SET end_ms=MAX(end_ms, excluded.end_ms)",
                params![start, now],
            )?;
            self.open = Some(start);
        } else if let Some(start) = self.open.take() {
            conn.execute(
                "UPDATE idle_span SET end_ms=MAX(end_ms, ?2) WHERE start_ms=?1",
                params![start, now - idle_ms],
            )?;
        }
        Ok(())
    }
}

/// The stored span that starts where `start` does, give or take the jitter of a sample: the one a
/// restarted daemon is looking at again.
fn same_span(conn: &Connection, start: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT start_ms FROM idle_span WHERE start_ms BETWEEN ?1 AND ?2 LIMIT 1",
            params![start - SAME_START_MS, start + SAME_START_MS],
            |r| r.get(0),
        )
        .optional()?)
}

/// `meta` key where the daemon notes whether it could read the idle time (`ok` or `none`), so that
/// `eye doctor`, which runs in a terminal, reports what the daemon itself can do.
pub const SENSOR_KEY: &str = "clock_idle_sensor";

/// What `eye doctor` knows about the daemon's idle sampling.
pub enum Daemon {
    /// No daemon is running: nothing is being sampled.
    Stopped,
    /// The daemon runs; it last noted that it could (`Some(true)`) or could not (`Some(false)`)
    /// read the idle time, or has not tried yet.
    Running(Option<bool>),
}

/// The state the daemon noted in `meta`, as `Daemon::Running` wants it.
pub fn noted(conn: &Connection) -> Option<bool> {
    let v: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key=?1", [SENSOR_KEY], |r| {
            r.get(0)
        })
        .optional()
        .ok()
        .flatten();
    v.map(|v| v == "ok")
}

/// The `eye doctor` line about the idle sensor: whether the daemon can say how long it has been
/// idle, and whether it is on. The daemon's own note counts; this process's reading only says what
/// a terminal sees, which is why it is used when the daemon is not running.
pub fn checks(cfg: &Config, sys: &dyn Sys, daemon: Daemon) -> Vec<Check> {
    if !cfg.clock.enabled {
        return vec![];
    }
    let here = idle_ms(sys).is_some();
    vec![match (cfg.clock.idle, daemon) {
        (false, _) => Check::ok(if here {
            "idle sensor: available, off (set [clock] idle = true to use it)"
        } else {
            "idle sensor: not available on this system, off"
        }),
        (true, Daemon::Running(Some(true))) => {
            Check::ok("idle sensor: on, the daemon reads idle time (breaks come from real idle time)")
        }
        (true, Daemon::Running(Some(false))) => Check::warn(
            "idle sensor: on in [clock], but the daemon cannot read idle time; breaks come from gaps between prompts",
        ),
        (true, Daemon::Running(None)) => Check::ok(
            "idle sensor: on, the daemon has not tried to read idle time yet (it does once a minute)",
        ),
        (true, Daemon::Stopped) => Check::warn(if here {
            "idle sensor: on, and this system can report idle time, but the daemon is not running, so none is recorded"
        } else {
            "idle sensor: on in [clock], but this system cannot report idle time; breaks come from gaps between prompts"
        }),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const MIN: i64 = 60_000;

    fn spans(db: &Db) -> Vec<(i64, i64)> {
        db.conn
            .prepare("SELECT start_ms, end_ms FROM idle_span ORDER BY start_ms")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn a_span_opens_grows_and_closes_where_the_input_came_back() {
        let db = Db::open_memory().unwrap();
        let mut s = Sampler::default();
        // the user stopped at minute 100; samples every minute
        s.sample(&db.conn, 103 * MIN, 3 * MIN).unwrap();
        assert!(spans(&db).is_empty(), "under five minutes is not a span");
        s.sample(&db.conn, 105 * MIN, 5 * MIN).unwrap();
        s.sample(&db.conn, 106 * MIN, 6 * MIN + 40).unwrap();
        assert_eq!(spans(&db), [(100 * MIN, 106 * MIN)]);
        // back at 107:30, seen at the sample at 108
        s.sample(&db.conn, 108 * MIN, 30_000).unwrap();
        assert_eq!(spans(&db), [(100 * MIN, 107 * MIN + 30_000)]);
        // nothing is open any more: another quiet sample changes nothing
        s.sample(&db.conn, 109 * MIN, 30_000).unwrap();
        assert_eq!(spans(&db).len(), 1);
    }

    #[test]
    fn a_restart_in_the_middle_of_a_span_continues_it() {
        let db = Db::open_memory().unwrap();
        Sampler::default()
            .sample(&db.conn, 110 * MIN, 10 * MIN)
            .unwrap();
        // a new sampler, one minute later; the start differs by a few milliseconds
        let mut again = Sampler::default();
        again
            .sample(&db.conn, 111 * MIN + 12, 11 * MIN + 20)
            .unwrap();
        assert_eq!(spans(&db), [(100 * MIN, 111 * MIN + 12)]);
    }

    #[test]
    fn two_stretches_are_two_spans() {
        let db = Db::open_memory().unwrap();
        let mut s = Sampler::default();
        s.sample(&db.conn, 20 * MIN, 10 * MIN).unwrap();
        s.sample(&db.conn, 21 * MIN, 0).unwrap();
        s.sample(&db.conn, 60 * MIN, 30 * MIN).unwrap();
        assert_eq!(spans(&db), [(10 * MIN, 21 * MIN), (30 * MIN, 60 * MIN)]);
    }

    struct Machine(bool);
    impl Sys for Machine {
        fn macos(&self) -> bool {
            true
        }
        fn output(&self, _: &str, _: &[&str]) -> Option<String> {
            self.0.then(|| "\"HIDIdleTime\" = 1000000000".to_string())
        }
        fn now_ms(&self) -> i64 {
            0
        }
    }

    #[test]
    fn doctor_reports_what_the_daemon_can_sample() {
        let cfg = |t: &str| Config::parse(t).unwrap();
        let line = |t: &str, here: bool, d: Daemon| {
            let c = checks(&cfg(t), &Machine(here), d);
            (c[0].ok, c[0].msg.clone())
        };
        let on = "[clock]\nidle = true";
        assert!(line("", true, Daemon::Stopped).1.contains("available, off"));
        assert!(line(on, false, Daemon::Running(Some(true)))
            .1
            .contains("the daemon reads"));
        // the terminal can read it but the daemon cannot: the daemon's word counts
        let (ok, msg) = line(on, true, Daemon::Running(Some(false)));
        assert!(!ok && msg.contains("the daemon cannot read idle time"));
        assert!(line(on, true, Daemon::Running(None)).0);
        let (ok, msg) = line(on, true, Daemon::Stopped);
        assert!(!ok && msg.contains("daemon is not running"));
        let (ok, msg) = line(on, false, Daemon::Stopped);
        assert!(!ok && msg.contains("cannot report idle time"));
        assert!(
            line("", false, Daemon::Stopped).0,
            "off and unavailable is nothing to fix"
        );
        assert!(checks(
            &cfg("[clock]\nenabled = false"),
            &Machine(true),
            Daemon::Stopped
        )
        .is_empty());
    }

    #[test]
    fn the_daemons_note_is_read_back() {
        let db = Db::open_memory().unwrap();
        assert_eq!(noted(&db.conn), None);
        crate::db::meta_set(&db.conn, SENSOR_KEY, "none").unwrap();
        assert_eq!(noted(&db.conn), Some(false));
        crate::db::meta_set(&db.conn, SENSOR_KEY, "ok").unwrap();
        assert_eq!(noted(&db.conn), Some(true));
    }
}
