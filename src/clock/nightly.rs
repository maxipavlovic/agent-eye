//! What the daemon does for the clock: sample the idle time once a minute while `[clock] idle` is
//! on, and run the corrections pass once a night, in the night window when the night shift is on
//! and from 04:00 otherwise. A night is tried once: a pass that fails (the agent is missing, the
//! plan limit is hit) is not repeated until the next night. A pass does not start while you are at
//! work (it spends plan quota next to your live session); it stays due and starts when you stop.
//! The settings are read again for every pass and sample, so a change on the board counts at once.

use super::idle::{Sampler, SENSOR_KEY};
use super::pass;
use crate::agents;
use crate::config::Config;
use crate::config_clock::ClockCfg;
use crate::daemon::log;
use crate::db::{meta_set, Db};
use crate::night::clock::{in_window, local_date, night_of, OffsetFn, Window};
use crate::paths::Paths;
use crate::platform::idle::{idle_ms, RealSys};
use crate::util::now_ms;
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `meta` key holding the night the pass last started for.
const DONE_KEY: &str = "clock_detect_night";
const IDLE_EVERY: Duration = Duration::from_secs(60);
const DUE_EVERY: Duration = Duration::from_secs(60);
/// You are at work when you sent a prompt this recently...
const BUSY_PROMPT_MS: i64 = 10 * 60_000;
/// ...or, with the idle sensor on, had input this recently.
const BUSY_IDLE_MS: i64 = 2 * 60_000;
/// Without the night shift, the pass runs from this local hour on.
const START_HOUR: usize = 4;

/// The night the pass is due for now, or None. `done` is the night it last started for.
pub fn due(cfg: &Config, now: i64, done: Option<&str>, off: OffsetFn) -> Option<String> {
    if !ClockCfg::llm_on(cfg) {
        return None;
    }
    let night = if cfg.night.enabled {
        let w = Window::parse(&cfg.night.window).unwrap_or_default();
        in_window(&w, now, off).then(|| night_of(&w, now, off))?
    } else {
        (super::bands::local_hour(now, off) >= START_HOUR).then(|| local_date(now, off))?
    };
    (done != Some(night.as_str())).then_some(night)
}

/// Is the user at work: a prompt in the last ten minutes, or (`idle` is how long the machine has
/// been idle, when the sensor is on) input in the last two.
pub fn user_busy(conn: &Connection, now: i64, idle: Option<i64>) -> bool {
    let prompted = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM turn WHERE kind='prompt' AND ts>=?1)",
            [now - BUSY_PROMPT_MS],
            |r| r.get(0),
        )
        .unwrap_or(false);
    prompted || idle.is_some_and(|ms| ms < BUSY_IDLE_MS)
}

/// The clock's part of the daemon's loop.
pub struct Clock {
    sampler: Sampler,
    last_idle: Option<Instant>,
    last_due: Option<Instant>,
    busy: Arc<AtomicBool>,
    /// The last idle reading and when it was taken.
    reading: Option<(Instant, i64)>,
    /// What was last noted for `eye doctor` (see `SENSOR_KEY`).
    noted: Option<bool>,
}

impl Clock {
    pub fn new() -> Clock {
        Clock {
            sampler: Sampler::default(),
            last_idle: None,
            last_due: None,
            busy: Arc::new(AtomicBool::new(false)),
            reading: None,
            noted: None,
        }
    }

    /// Called every tick; does something at most once a minute. `cfg` is what the daemon started
    /// with: the file is read again before anything is done.
    pub fn tick(&mut self, db: &Db, paths: &Paths, cfg: &Config) {
        let every = |last: Option<Instant>, d: Duration| last.is_none_or(|t| t.elapsed() >= d);
        let (idle_due, pass_due) = (
            every(self.last_idle, IDLE_EVERY),
            every(self.last_due, DUE_EVERY),
        );
        if !idle_due && !pass_due {
            return;
        }
        let cfg = cfg.fresh(&paths.config_path());
        if idle_due {
            self.last_idle = Some(Instant::now());
            self.sample(db, paths, &cfg);
        }
        if pass_due {
            self.last_due = Some(Instant::now());
            if cfg.clock.enabled {
                self.start_pass(db, paths, &cfg);
            }
        }
    }

    /// One idle sample, when the sensor is on; notes for `eye doctor` whether the daemon can read it.
    fn sample(&mut self, db: &Db, paths: &Paths, cfg: &Config) {
        self.reading = None;
        if !cfg.clock.enabled || !cfg.clock.idle {
            return;
        }
        let reading = idle_ms(&RealSys);
        if self.noted != Some(reading.is_some()) {
            let note = if reading.is_some() { "ok" } else { "none" };
            if db.meta_set(SENSOR_KEY, note).is_ok() {
                self.noted = Some(reading.is_some());
            }
        }
        let Some(ms) = reading else { return };
        self.reading = Some((Instant::now(), ms));
        if let Err(e) = self.sampler.sample(&db.conn, now_ms(), ms) {
            log(paths, &format!("idle sample: {e:#}"));
        }
    }

    fn start_pass(&self, db: &Db, paths: &Paths, cfg: &Config) {
        let done = db.meta_get(DONE_KEY).ok().flatten();
        let now = now_ms();
        let Some(night) = due(
            cfg,
            now,
            done.as_deref(),
            &crate::worked::summary::local_offset_ms,
        ) else {
            return;
        };
        // the night stays due: it is not marked done, so the next minute asks again
        let idle = self
            .reading
            .map(|(at, ms)| ms + at.elapsed().as_millis() as i64);
        if user_busy(&db.conn, now, idle) {
            return;
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = meta_set(&db.conn, DONE_KEY, &night);
        let (paths, cfg, busy) = (paths.clone(), cfg.clone(), self.busy.clone());
        std::thread::spawn(move || {
            round(&paths, &cfg);
            busy.store(false, Ordering::SeqCst);
        });
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

/// One corrections pass. Failures and what it did go to the log.
fn round(paths: &Paths, cfg: &Config) {
    let r = Db::open(&paths.db_path()).and_then(|db| {
        pass::run(&db, paths, cfg, &agents::enabled(cfg), now_ms(), &mut |l| {
            log(paths, &format!("corrections: {l}"))
        })
    });
    match r {
        Ok(o) => log(
            paths,
            &format!(
                "corrections: {} judged, {} corrections, {} pending{}",
                o.sent,
                o.corrections,
                o.pending,
                o.stopped
                    .map(|s| format!(", stopped: {s}"))
                    .unwrap_or_default()
            ),
        ),
        Err(e) => log(paths, &format!("corrections: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::night::clock::test_ms;

    fn plus2(_: i64) -> i64 {
        2 * 3_600_000
    }
    fn at(day: &str, hm: &str) -> i64 {
        test_ms(day, hm, 2)
    }
    fn cfg(t: &str) -> Config {
        Config::parse(t).unwrap()
    }

    #[test]
    fn without_the_night_shift_it_is_due_from_four_in_the_morning_once_a_day() {
        let c = cfg("");
        assert_eq!(due(&c, at("2026-11-26", "03:59"), None, &plus2), None);
        assert_eq!(
            due(&c, at("2026-11-26", "04:00"), None, &plus2).as_deref(),
            Some("2026-11-26")
        );
        assert_eq!(
            due(&c, at("2026-11-26", "15:00"), None, &plus2).as_deref(),
            Some("2026-11-26")
        );
        assert_eq!(
            due(&c, at("2026-11-26", "15:00"), Some("2026-11-26"), &plus2),
            None
        );
        assert_eq!(
            due(&c, at("2026-11-27", "04:00"), Some("2026-11-26"), &plus2).as_deref(),
            Some("2026-11-27")
        );
    }

    #[test]
    fn with_the_night_shift_it_is_due_inside_its_window_once_a_night() {
        let c = cfg("[night]\nenabled = true\nwindow = \"23:00-05:00\"");
        assert_eq!(due(&c, at("2026-11-26", "22:59"), None, &plus2), None);
        assert_eq!(
            due(&c, at("2026-11-26", "23:30"), None, &plus2).as_deref(),
            Some("2026-11-26")
        );
        // after midnight it is the same night
        assert_eq!(
            due(&c, at("2026-11-27", "01:00"), Some("2026-11-26"), &plus2),
            None
        );
        assert_eq!(due(&c, at("2026-11-27", "05:00"), None, &plus2), None);
    }

    #[test]
    fn it_is_never_due_when_judging_is_off() {
        let now = at("2026-11-26", "10:00");
        assert_eq!(due(&cfg("[clock]\nllm = false"), now, None, &plus2), None);
        assert_eq!(
            due(&cfg("[clock]\nenabled = false"), now, None, &plus2),
            None
        );
        assert_eq!(
            due(&cfg("[summary]\nenabled = false"), now, None, &plus2),
            None
        );
    }

    fn db_with_prompt_at(ts: i64) -> crate::db::Db {
        let db = crate::db::Db::open_memory().unwrap();
        db.conn
            .execute("INSERT INTO session(id, agent) VALUES('s','claude')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO turn(session_id, ts, kind, n) VALUES('s',?1,'prompt',5)",
                [ts],
            )
            .unwrap();
        db
    }

    #[test]
    fn a_recent_prompt_or_recent_input_means_you_are_at_work() {
        let now = 100 * 60_000;
        let db = db_with_prompt_at(now - 9 * 60_000);
        assert!(user_busy(&db.conn, now, None));
        let old = db_with_prompt_at(now - 11 * 60_000);
        assert!(!user_busy(&old.conn, now, None));
        // the idle sensor: input a minute ago is work, ten minutes of quiet is not
        assert!(user_busy(&old.conn, now, Some(60_000)));
        assert!(!user_busy(&old.conn, now, Some(10 * 60_000)));
        let empty = crate::db::Db::open_memory().unwrap();
        assert!(!user_busy(&empty.conn, now, None));
    }
}
