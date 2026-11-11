//! The words for a time: `tomorrow`, `mon`..`sun`, `3d`, `1w`, `4h`, `2026-11-20`, `2026-11-20 14:00`.
//! One parser for the command line, the board and the picker. A day without a time is 9:00.

use anyhow::{bail, ensure, Result};
use chrono::{
    DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday,
};

/// The hour of the day the words mean when they name a day and no time.
const MORNING_HOUR: i64 = 9;

/// The most days, weeks or hours that a relative time may reach ahead, so a typo cannot overflow.
const MAX_AHEAD: i64 = 10_000;

const HELP: &str =
    "say when as tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20 or 2026-11-20 14:00";

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];

/// The time `text` names, in Unix ms, in local time and after `now_ms`.
pub fn parse_local(text: &str, now_ms: i64) -> Result<i64> {
    let now = Local
        .timestamp_millis_opt(now_ms)
        .single()
        .ok_or_else(|| anyhow::anyhow!("the clock is out of range"))?;
    parse(text, &now)
}

/// The time `text` names, in Unix ms, in the zone of `now` and after it.
pub fn parse<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Result<i64> {
    let t = text.trim().to_lowercase();
    ensure!(!t.is_empty(), "{HELP}");
    let tz = now.timezone();
    let today = now.date_naive();
    let nine = |d: NaiveDate| at(&tz, morning(d));
    let when = if t == "tomorrow" {
        nine(today + Duration::days(1))
    } else if let Some(day) = weekday(&t) {
        let ahead = (day.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
        nine(today + Duration::days(if ahead == 0 { 7 } else { i64::from(ahead) }))
    } else if let Some((n, unit)) = relative(&t) {
        match unit {
            'm' => now.clone() + Duration::minutes(n),
            'h' => now.clone() + Duration::hours(n),
            // days and weeks keep the time of day, whatever the clocks did in between
            'd' => at(&tz, now.naive_local() + Duration::days(n)),
            _ => at(&tz, now.naive_local() + Duration::days(7 * n)),
        }
    } else if let Some(d) = absolute(&t) {
        at(&tz, d)
    } else {
        bail!("cannot read '{}': {HELP}", text.trim());
    };
    ensure!(when > *now, "{} has already passed", text.trim());
    Ok(when.timestamp_millis())
}

fn weekday(t: &str) -> Option<Weekday> {
    WEEKDAYS
        .iter()
        .find(|(name, _)| t.len() >= 3 && name.starts_with(t))
        .map(|(_, d)| *d)
}

/// `4h` as (4, 'h'); the unit is m, h, d or w.
fn relative(t: &str) -> Option<(i64, char)> {
    let unit = t.chars().last().filter(|c| "mhdw".contains(*c))?;
    let n: i64 = t[..t.len() - 1].parse().ok()?;
    (1..=MAX_AHEAD).contains(&n).then_some((n, unit))
}

/// A date, with a time or at the default one.
fn absolute(t: &str) -> Option<NaiveDateTime> {
    // the lower-cased text of `2026-11-20T14:00` has a `t` where a space may be
    NaiveDateTime::parse_from_str(&t.replacen('t', " ", 1), "%Y-%m-%d %H:%M")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(t, "%Y-%m-%d").ok().map(morning))
}

/// A day at the default time.
fn morning(d: NaiveDate) -> NaiveDateTime {
    d.and_time(NaiveTime::MIN + Duration::hours(MORNING_HOUR))
}

/// A wall-clock time in a zone. When the clocks go back and it happens twice, the first one; when
/// they go forward and it never happens, the same time an hour later.
fn at<Tz: TimeZone>(tz: &Tz, wall: NaiveDateTime) -> DateTime<Tz> {
    let first = |w: NaiveDateTime| tz.from_local_datetime(&w).earliest();
    first(wall)
        .or_else(|| first(wall + Duration::hours(1)))
        .unwrap_or_else(|| tz.from_utc_datetime(&wall))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, LocalResult, Offset, Utc};

    /// Central European time with its real rules for 2026: +01:00, +02:00 from 29 March 01:00 UTC
    /// (02:00 becomes 03:00) to 25 October 01:00 UTC (03:00 becomes 02:00).
    #[derive(Clone, Copy, Debug)]
    struct Cet;

    fn forward() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
    }
    fn back() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
    }
    fn offset_at_utc(utc: &NaiveDateTime) -> FixedOffset {
        let hours = if *utc >= forward() && *utc < back() {
            2
        } else {
            1
        };
        FixedOffset::east_opt(hours * 3600).unwrap()
    }

    #[derive(Clone, Copy, Debug)]
    struct CetOffset(FixedOffset);
    impl Offset for CetOffset {
        fn fix(&self) -> FixedOffset {
            self.0
        }
    }

    impl TimeZone for Cet {
        type Offset = CetOffset;
        fn from_offset(_: &CetOffset) -> Cet {
            Cet
        }
        fn offset_from_local_date(&self, d: &NaiveDate) -> LocalResult<CetOffset> {
            self.offset_from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, l: &NaiveDateTime) -> LocalResult<CetOffset> {
            let as_winter = *l - Duration::hours(1);
            let as_summer = *l - Duration::hours(2);
            let winter = offset_at_utc(&as_winter).local_minus_utc() == 3600;
            let summer = offset_at_utc(&as_summer).local_minus_utc() == 7200;
            match (winter, summer) {
                (true, true) => LocalResult::Ambiguous(
                    CetOffset(offset_at_utc(&as_summer)),
                    CetOffset(offset_at_utc(&as_winter)),
                ),
                (true, false) => LocalResult::Single(CetOffset(offset_at_utc(&as_winter))),
                (false, true) => LocalResult::Single(CetOffset(offset_at_utc(&as_summer))),
                (false, false) => LocalResult::None,
            }
        }
        fn offset_from_utc_date(&self, d: &NaiveDate) -> CetOffset {
            self.offset_from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> CetOffset {
            CetOffset(offset_at_utc(utc))
        }
    }

    impl std::fmt::Display for CetOffset {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            self.0.fmt(f)
        }
    }

    /// A moment as local wall-clock text in `Cet`.
    fn show(ms: i64) -> String {
        Cet.timestamp_millis_opt(ms)
            .unwrap()
            .format("%a %Y-%m-%d %H:%M")
            .to_string()
    }

    fn now(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Cet> {
        Cet.with_ymd_and_hms(y, m, d, h, mi, 0).earliest().unwrap()
    }

    fn at_(text: &str, from: &DateTime<Cet>) -> String {
        show(parse(text, from).unwrap())
    }

    #[test]
    fn a_day_without_a_time_is_nine_in_the_morning() {
        let n = now(2026, 11, 11, 15, 30); // a Wednesday
        assert_eq!(at_("tomorrow", &n), "Thu 2026-11-12 09:00");
        assert_eq!(at_("2026-11-20", &n), "Fri 2026-11-20 09:00");
        assert_eq!(at_("2026-11-20 14:00", &n), "Fri 2026-11-20 14:00");
        assert_eq!(at_("2026-11-20T14:05", &n), "Fri 2026-11-20 14:05");
    }

    #[test]
    fn a_weekday_is_the_next_such_day_never_today() {
        let n = now(2026, 11, 11, 8, 0); // a Wednesday
        assert_eq!(at_("fri", &n), "Fri 2026-11-13 09:00");
        assert_eq!(at_("Monday", &n), "Mon 2026-11-16 09:00");
        assert_eq!(at_("wed", &n), "Wed 2026-11-18 09:00");
        assert_eq!(at_("tues", &n), "Tue 2026-11-17 09:00");
        assert_eq!(at_("sun", &n), "Sun 2026-11-15 09:00");
    }

    #[test]
    fn relative_times_count_from_now() {
        let n = now(2026, 11, 11, 15, 30);
        assert_eq!(at_("30m", &n), "Wed 2026-11-11 16:00");
        assert_eq!(at_("4h", &n), "Wed 2026-11-11 19:30");
        assert_eq!(at_("3d", &n), "Sat 2026-11-14 15:30");
        assert_eq!(at_("1w", &n), "Wed 2026-11-18 15:30");
        assert_eq!(at_(" 2D ", &n), "Fri 2026-11-13 15:30");
    }

    #[test]
    fn days_keep_the_clock_time_across_a_change_of_clocks_and_hours_do_not() {
        // the night of 29 March 2026 is one hour short
        let n = now(2026, 3, 28, 10, 0);
        assert_eq!(at_("1d", &n), "Sun 2026-03-29 10:00");
        assert_eq!(at_("24h", &n.clone()), "Sun 2026-03-29 11:00");
        assert_eq!(at_("tomorrow", &n), "Sun 2026-03-29 09:00");
        // the night of 25 October 2026 is one hour long
        let n = now(2026, 10, 24, 10, 0);
        assert_eq!(at_("1d", &n), "Sun 2026-10-25 10:00");
        assert_eq!(at_("24h", &n), "Sun 2026-10-25 09:00");
    }

    #[test]
    fn a_time_that_does_not_exist_moves_on_an_hour_and_one_that_happens_twice_is_the_first() {
        let n = now(2026, 3, 20, 10, 0);
        assert_eq!(at_("2026-03-29 02:30", &n), "Sun 2026-03-29 03:30");
        let n = now(2026, 10, 20, 10, 0);
        let first = parse("2026-10-25 02:30", &n).unwrap();
        let second = first + 3_600_000;
        assert_eq!(show(first), "Sun 2026-10-25 02:30");
        assert_eq!(show(second), "Sun 2026-10-25 02:30");
        assert_eq!(
            Utc.timestamp_millis_opt(first)
                .unwrap()
                .format("%H:%M")
                .to_string(),
            "00:30"
        );
    }

    #[test]
    fn a_time_in_the_past_or_unreadable_words_are_refused_with_the_accepted_forms() {
        let n = now(2026, 11, 11, 15, 30);
        let e = parse("2026-11-11 15:30", &n).unwrap_err().to_string();
        assert!(e.contains("already passed"), "{e}");
        assert!(parse("2020-01-01", &n).is_err());
        for bad in [
            "",
            "  ",
            "soon",
            "0h",
            "-3d",
            "3x",
            "2026-13-40",
            "10001d",
            "mo",
        ] {
            let e = parse(bad, &n).unwrap_err().to_string();
            assert!(e.contains("tomorrow") || e.contains("passed"), "{bad}: {e}");
        }
    }

    #[test]
    fn local_time_is_the_machines_own() {
        let now_ms = crate::util::now_ms();
        let at = parse_local("4h", now_ms).unwrap();
        assert!((at - now_ms - 4 * 3_600_000).abs() < 1000);
    }
}
