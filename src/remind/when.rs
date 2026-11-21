//! The words for a time: `tomorrow`, `mon`..`sun`, `3d`, `1w`, `4h`, `2026-11-20`, `2026-11-20 14:00`.
//! One parser for the command line, the board and the picker. A day without a time is 9:00.

use anyhow::{anyhow, bail, ensure, Result};
use jiff::{
    civil::{Date, DateTime, Weekday},
    tz::TimeZone,
    Span, Timestamp, Zoned,
};

/// The hour of the day the words mean when they name a day and no time.
const MORNING_HOUR: i8 = 9;

/// The most days, weeks or hours that a relative time may reach ahead, so a typo cannot overflow.
const MAX_AHEAD: i64 = 10_000;

const HELP: &str =
    "say when as tomorrow, mon..sun, 30m, 4h, 3d, 1w, 2026-11-20 or 2026-11-20 14:00";

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Monday),
    ("tuesday", Weekday::Tuesday),
    ("wednesday", Weekday::Wednesday),
    ("thursday", Weekday::Thursday),
    ("friday", Weekday::Friday),
    ("saturday", Weekday::Saturday),
    ("sunday", Weekday::Sunday),
];

/// The time `text` names, in Unix ms, in local time and after `now_ms`.
pub fn parse_local(text: &str, now_ms: i64) -> Result<i64> {
    let now = Timestamp::from_millisecond(now_ms)
        .map_err(|_| anyhow!("the clock is out of range"))?
        .to_zoned(TimeZone::system());
    parse(text, &now)
}

/// The time `text` names, in Unix ms, in the zone of `now` and after it.
pub fn parse(text: &str, now: &Zoned) -> Result<i64> {
    let t = text.trim().to_lowercase();
    ensure!(!t.is_empty(), "{HELP}");
    let tz = now.time_zone();
    let today = now.date();
    let out_of_range = || anyhow!("{} is out of range", text.trim());
    let days = |from: DateTime, n: i64| -> Result<DateTime> {
        let span = Span::new().try_days(n).map_err(|_| out_of_range())?;
        from.checked_add(span).map_err(|_| out_of_range())
    };
    let nine = |d: Date| at(tz, morning(d));
    let when = if t == "tomorrow" {
        nine(days(today.into(), 1)?.date())?
    } else if let Some(day) = weekday(&t) {
        let ahead = (i64::from(day.to_monday_zero_offset()) + 7
            - i64::from(today.weekday().to_monday_zero_offset()))
            % 7;
        nine(days(today.into(), if ahead == 0 { 7 } else { ahead })?.date())?
    } else if let Some((n, unit)) = relative(&t) {
        match unit {
            'm' => now.checked_add(Span::new().minutes(n)),
            'h' => now.checked_add(Span::new().hours(n)),
            // days and weeks keep the time of day, whatever the clocks did in between
            'd' => return finish(text, now, at(tz, days(now.datetime(), n)?)?),
            _ => return finish(text, now, at(tz, days(now.datetime(), 7 * n)?)?),
        }
        .map_err(|_| out_of_range())?
    } else if let Some(d) = absolute(&t) {
        at(tz, d)?
    } else {
        bail!("cannot read '{}': {HELP}", text.trim());
    };
    finish(text, now, when)
}

fn finish(text: &str, now: &Zoned, when: Zoned) -> Result<i64> {
    ensure!(when > *now, "{} has already passed", text.trim());
    Ok(when.timestamp().as_millisecond())
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
fn absolute(t: &str) -> Option<DateTime> {
    // the lower-cased text of `2026-11-20T14:00` has a `t` where a space may be
    DateTime::strptime("%Y-%m-%d %H:%M", t.replacen('t', " ", 1))
        .ok()
        .or_else(|| Date::strptime("%Y-%m-%d", t).ok().map(morning))
}

/// A day at the default time.
fn morning(d: Date) -> DateTime {
    d.at(MORNING_HOUR, 0, 0, 0)
}

/// A wall-clock time in a zone. When the clocks go back and it happens twice, the first one; when
/// they go forward and it never happens, the same time an hour later.
fn at(tz: &TimeZone, wall: DateTime) -> Result<Zoned> {
    let first = |w: DateTime| crate::util::wall_earliest(tz, w);
    first(wall)
        .or_else(|| first(wall.checked_add(Span::new().hours(1)).ok()?))
        .ok_or_else(|| anyhow!("{wall} is out of range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spain's real rules, from the bundled tz database: +01:00, +02:00 from 29 March 2026 01:00
    /// UTC (02:00 becomes 03:00) to 25 October 2026 01:00 UTC (03:00 becomes 02:00).
    fn madrid() -> TimeZone {
        TimeZone::get("Europe/Madrid").unwrap()
    }

    /// A moment as local wall-clock text in Madrid.
    fn show(ms: i64) -> String {
        Timestamp::from_millisecond(ms)
            .unwrap()
            .to_zoned(madrid())
            .strftime("%a %Y-%m-%d %H:%M")
            .to_string()
    }

    fn now(y: i16, m: i8, d: i8, h: i8, mi: i8) -> Zoned {
        crate::util::wall_earliest(&madrid(), jiff::civil::date(y, m, d).at(h, mi, 0, 0)).unwrap()
    }

    fn at_(text: &str, from: &Zoned) -> String {
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
        assert_eq!(at_("24h", &n), "Sun 2026-03-29 11:00");
        assert_eq!(at_("tomorrow", &n), "Sun 2026-03-29 09:00");
        // the night of 25 October 2026 is one hour long
        let n = now(2026, 10, 24, 10, 0);
        assert_eq!(at_("1d", &n), "Sun 2026-10-25 10:00");
        assert_eq!(at_("24h", &n), "Sun 2026-10-25 09:00");
    }

    #[test]
    fn tomorrow_at_nine_is_nine_on_the_change_days_and_the_days_around_them() {
        for (from, want) in [
            (now(2026, 3, 28, 22, 0), "Sun 2026-03-29 09:00"),
            (now(2026, 3, 29, 12, 0), "Mon 2026-03-30 09:00"),
            (now(2026, 3, 29, 1, 30), "Mon 2026-03-30 09:00"),
            (now(2026, 10, 24, 22, 0), "Sun 2026-10-25 09:00"),
            (now(2026, 10, 25, 12, 0), "Mon 2026-10-26 09:00"),
        ] {
            assert_eq!(at_("tomorrow", &from), want);
        }
        // the distance in real time is 23, 25, 24 and 24 hours, and never 24 by force
        let hours = |a: &Zoned, text: &str| {
            (parse(text, a).unwrap() - a.timestamp().as_millisecond()) as f64 / 3_600_000.0
        };
        assert_eq!(hours(&now(2026, 3, 28, 9, 0), "tomorrow"), 24.0 - 1.0);
        assert_eq!(hours(&now(2026, 10, 24, 9, 0), "tomorrow"), 24.0 + 1.0);
        assert_eq!(hours(&now(2026, 3, 29, 9, 0), "tomorrow"), 24.0);
        assert_eq!(hours(&now(2026, 3, 27, 9, 0), "tomorrow"), 24.0);
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
            Timestamp::from_millisecond(first)
                .unwrap()
                .to_zoned(TimeZone::UTC)
                .strftime("%H:%M")
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
