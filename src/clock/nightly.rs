//! What the daemon does for the clock: sample the idle time once a minute while `[clock] idle` is
//! on, and run the corrections pass once a night, in the night window when the night shift is on
//! and from 04:00 otherwise. A night is tried once: a pass that fails (the agent is missing, the
//! plan limit is hit) is not repeated until the next night.

use super::idle::Sampler;
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `meta` key holding the night the pass last started for.
const DONE_KEY: &str = "clock_detect_night";
const IDLE_EVERY: Duration = Duration::from_secs(60);
const DUE_EVERY: Duration = Duration::from_secs(60);
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

/// The clock's part of the daemon's loop.
pub struct Clock {
    sampler: Sampler,
    last_idle: Option<Instant>,
    last_due: Option<Instant>,
    busy: Arc<AtomicBool>,
}

impl Clock {
    pub fn new() -> Clock {
        Clock {
            sampler: Sampler::default(),
            last_idle: None,
            last_due: None,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Called every tick; does something at most once a minute.
    pub fn tick(&mut self, db: &Db, paths: &Paths, cfg: &Config) {
        if !cfg.clock.enabled {
            return;
        }
        if cfg.clock.idle && self.last_idle.is_none_or(|t| t.elapsed() >= IDLE_EVERY) {
            self.last_idle = Some(Instant::now());
            if let Some(ms) = idle_ms(&RealSys) {
                if let Err(e) = self.sampler.sample(&db.conn, now_ms(), ms) {
                    log(paths, &format!("idle sample: {e:#}"));
                }
            }
        }
        if self.last_due.is_none_or(|t| t.elapsed() >= DUE_EVERY) {
            self.last_due = Some(Instant::now());
            self.start_pass(db, paths, cfg);
        }
    }

    fn start_pass(&self, db: &Db, paths: &Paths, cfg: &Config) {
        let done = db.meta_get(DONE_KEY).ok().flatten();
        let Some(night) = due(
            cfg,
            now_ms(),
            done.as_deref(),
            &crate::worked::summary::local_offset_ms,
        ) else {
            return;
        };
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
}
