//! A session's hours as local days, for the card and the terminal. The hours are UTC, so a change
//! of time zone or a daylight saving change never moves what is stored; the day a message falls on
//! is worked out here, with the offset the machine has at that time.

use super::{hour_start, store, Hour};
use anyhow::Result;
use chrono::{Local, NaiveDate, Offset, TimeZone};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

/// The strip of days on a card shows at most this many calendar days, ending with the last one.
pub const WINDOW_DAYS: i64 = 60;
const DAY_MS: i64 = 86_400_000;

/// The offset from UTC of the local time zone at a Unix ms time, in ms.
pub type OffsetFn<'a> = &'a dyn Fn(i64) -> i64;

/// The offset of this machine's time zone at `utc_ms`.
pub fn local_offset_ms(utc_ms: i64) -> i64 {
    Local
        .timestamp_millis_opt(utc_ms)
        .earliest()
        .map_or(0, |d| i64::from(d.offset().fix().local_minus_utc()) * 1000)
}

/// One local day with work in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Day {
    /// `YYYY-MM-DD`.
    pub day: String,
    pub prompts: i64,
    pub active_ms: i64,
}

/// The days a session was worked on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Worked {
    /// The first and last calendar day of the strip (the first is later than the session's first
    /// day when the span is longer than the window).
    pub first_day: String,
    pub last_day: String,
    /// The days with work in them inside the strip, oldest first.
    pub days: Vec<Day>,
    /// Days with work over the whole session, and calendar days from its first to its last.
    pub days_count: i64,
    pub span_days: i64,
    /// Active time over the whole session.
    pub active_ms: i64,
    /// The time of the last message.
    pub last_at: i64,
    /// Calendar days of the span before the strip starts.
    pub more_days: i64,
}

fn local_day(utc_ms: i64, offset: OffsetFn) -> i64 {
    (utc_ms + offset(utc_ms)).div_euclid(DAY_MS)
}

/// `YYYY-MM-DD` of a day number (days since 1970-01-01).
pub fn day_name(day: i64) -> String {
    i32::try_from(day + 719_163)
        .ok()
        .and_then(NaiveDate::from_num_days_from_ce_opt)
        .map_or_else(|| "?".into(), |d| d.format("%Y-%m-%d").to_string())
}

/// Group hours into local days. `hours` must not be empty.
pub fn summarize(hours: &[(i64, Hour)], last_at: i64, offset: OffsetFn) -> Worked {
    let mut days: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut active_ms = 0;
    for (hour, h) in hours {
        let d = days
            .entry(local_day(hour_start(*hour), offset))
            .or_default();
        d.0 += h.prompts;
        d.1 += h.active_ms;
        active_ms += h.active_ms;
    }
    let first = days.keys().next().copied().unwrap_or_default();
    let last = days.keys().next_back().copied().unwrap_or_default();
    let span_days = last - first + 1;
    let start = first.max(last - WINDOW_DAYS + 1);
    Worked {
        first_day: day_name(start),
        last_day: day_name(last),
        days: days
            .iter()
            .filter(|(d, _)| **d >= start)
            .map(|(d, (prompts, active_ms))| Day {
                day: day_name(*d),
                prompts: *prompts,
                active_ms: *active_ms,
            })
            .collect(),
        days_count: days.len() as i64,
        span_days,
        active_ms,
        last_at,
        more_days: start - first,
    }
}

/// The days of a stored session; None while it has not been counted or has no messages.
pub fn of_session(conn: &Connection, sid: &str, offset: OffsetFn) -> Result<Option<Worked>> {
    Ok(store::load(conn, sid)?.map(|(hours, last)| summarize(&hours, last, offset)))
}

/// "7 h 40 m", "25 m", "under 1 m".
pub fn duration_text(ms: i64) -> String {
    let minutes = ms.max(0) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "under 1 m".into(),
        (0, m) => format!("{m} m"),
        (h, m) => format!("{h} h {m:02} m"),
    }
}

/// `today 14:20`, `yesterday 09:05` or `2026-11-03 09:05`: when `at` was, as local time.
fn when(at: i64, now: i64, offset: OffsetFn) -> String {
    let clock = (at + offset(at)).rem_euclid(DAY_MS) / 60_000;
    let day = local_day(at, offset);
    let name = match local_day(now, offset) - day {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        _ => day_name(day),
    };
    format!("{name} {:02}:{:02}", clock / 60, clock % 60)
}

/// What the terminal shows after "worked": `6 days · 7 h 40 m · last today 14:20`.
pub fn line(w: &Worked, now: i64, offset: OffsetFn) -> String {
    let days = if w.days_count == 1 {
        "1 day".to_string()
    } else {
        format!("{} days", w.days_count)
    };
    format!(
        "{days} · {} · last {}",
        duration_text(w.active_ms),
        when(w.last_at, now, offset)
    )
}

/// The line for the session `sid`, None while it has no days.
pub fn session_line(conn: &Connection, sid: &str, now: i64) -> Option<String> {
    of_session(conn, sid, &local_offset_ms)
        .ok()
        .flatten()
        .map(|w| line(&w, now, &local_offset_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;
    const MIN: i64 = 60_000;
    /// 2026-11-10 00:00 UTC, as hours since the epoch.
    const D0: i64 = 20_767 * 24;

    fn utc(_: i64) -> i64 {
        0
    }

    fn h(prompts: i64, active_min: i64) -> Hour {
        Hour {
            prompts,
            events: prompts * 2,
            active_ms: active_min * MIN,
        }
    }

    #[test]
    fn hours_group_into_days_with_their_totals() {
        let hours = [
            (D0 + 9, h(2, 30)),
            (D0 + 10, h(1, 20)),
            (D0 + 24 * 3 + 5, h(4, 40)),
        ];
        let w = summarize(&hours, (D0 + 24 * 3 + 5) * HOUR, &utc);
        assert_eq!(w.days_count, 2);
        assert_eq!(w.span_days, 4);
        assert_eq!(w.active_ms, 90 * MIN);
        assert_eq!(w.days[0].day, "2026-11-10");
        assert_eq!((w.days[0].prompts, w.days[0].active_ms), (3, 50 * MIN));
        assert_eq!(w.days[1].day, "2026-11-13");
        assert_eq!(
            (w.first_day.as_str(), w.last_day.as_str()),
            ("2026-11-10", "2026-11-13")
        );
        assert_eq!(w.more_days, 0);
    }

    #[test]
    fn a_long_span_shows_the_last_sixty_days_and_counts_the_rest() {
        let hours = [
            (D0, h(1, 5)),
            (D0 + 24 * 99, h(1, 5)),
            (D0 + 24 * 100, h(1, 5)),
        ];
        let w = summarize(&hours, (D0 + 24 * 100) * HOUR, &utc);
        assert_eq!(w.span_days, 101);
        assert_eq!(w.days_count, 3);
        assert_eq!(w.days.len(), 2);
        assert_eq!(w.more_days, 41);
        assert_eq!(w.active_ms, 15 * MIN);
    }

    #[test]
    fn the_local_day_follows_the_offset_and_a_daylight_saving_change() {
        // Offset +1 h until 2026-11-12 00:00 UTC, +2 h after it.
        let switch = (D0 + 48) * HOUR;
        let zone = move |t: i64| if t < switch { HOUR } else { 2 * HOUR };
        // 23:30 UTC on the 10th is already the 11th at +1 h; 22:30 UTC on the 12th is the 13th at +2 h.
        let hours = [
            (D0 + 23, h(1, 5)),
            (D0 + 24 * 3 - 2, h(1, 5)),
            (D0 + 24 * 2 - 1, h(1, 5)),
        ];
        let mut sorted = hours.to_vec();
        sorted.sort_by_key(|(t, _)| *t);
        let w = summarize(&sorted, (D0 + 24 * 3 - 2) * HOUR, &zone);
        let names: Vec<&str> = w.days.iter().map(|d| d.day.as_str()).collect();
        assert_eq!(names, ["2026-11-11", "2026-11-12", "2026-11-13"]);
        assert_eq!(w.span_days, 3);
    }

    #[test]
    fn the_line_says_days_time_and_the_last_message() {
        let hours = [(D0 + 9, h(2, 400)), (D0 + 24 * 2 + 14, h(1, 60))];
        let last = (D0 + 24 * 2 + 14) * HOUR + 20 * MIN;
        let w = summarize(&hours, last, &utc);
        assert_eq!(
            line(&w, last + 2 * HOUR, &utc),
            "2 days · 7 h 40 m · last today 14:20"
        );
        assert!(line(&w, last + 30 * HOUR, &utc).ends_with("last yesterday 14:20"));
        assert!(line(&w, last + 100 * HOUR, &utc).ends_with("last 2026-11-12 14:20"));
    }

    #[test]
    fn a_short_session_reads_in_minutes() {
        assert_eq!(duration_text(25 * MIN), "25 m");
        assert_eq!(duration_text(10_000), "under 1 m");
        assert_eq!(duration_text(61 * MIN), "1 h 01 m");
    }

    #[test]
    fn a_session_that_was_not_counted_has_no_line() {
        let db = crate::db::Db::open_memory().unwrap();
        assert_eq!(session_line(&db.conn, "nope", 0), None);
    }
}
