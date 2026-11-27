//! All the time logic of the night shift: the window, which night a moment belongs to, and local
//! dates. Everything works on Unix ms and an injected offset function (the machine's offset from
//! UTC at a time), so tests set the clock and the zone without touching the machine's.

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
const DAY_MIN: i64 = 1_440;

/// The offset from UTC, in ms, of a Unix ms time.
pub type OffsetFn<'a> = &'a dyn Fn(i64) -> i64;

/// The offset of this machine's time zone at `utc_ms` (the C library's `localtime_r`).
pub fn local_offset(utc_ms: i64) -> i64 {
    let secs = utc_ms.div_euclid(1000) as libc::time_t;
    // SAFETY: `tm` is plain data that `localtime_r` fills; both pointers are valid for the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return 0;
        }
        #[allow(clippy::useless_conversion)] // c_long is i32 on some targets
        let off = i64::from(tm.tm_gmtoff);
        off * 1000
    }
}

/// The time of day the window opens and closes, in minutes after local midnight. A window whose
/// end is before its start crosses midnight (`23:00-05:00`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            start: 60,
            end: 7 * 60,
        }
    }
}

fn parse_clock(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (i64, i64) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m) && m.to_string().len() <= 2).then_some(h * 60 + m)
}

fn clock_text(min: i64) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

impl Window {
    /// `01:00-07:00`. Start and end must differ.
    pub fn parse(s: &str) -> Result<Window, String> {
        let bad = || format!("'{s}' is not a window: write it as 01:00-07:00");
        let (a, b) = s.split_once('-').ok_or_else(bad)?;
        let (start, end) = (
            parse_clock(a).ok_or_else(bad)?,
            parse_clock(b).ok_or_else(bad)?,
        );
        if start == end {
            return Err("the window must start and end at different times".into());
        }
        Ok(Window { start, end })
    }

    pub fn text(&self) -> String {
        format!("{}-{}", clock_text(self.start), clock_text(self.end))
    }

    /// Does the window cover this local minute of the day?
    pub fn covers(&self, min: i64) -> bool {
        if self.start < self.end {
            (self.start..self.end).contains(&min)
        } else {
            min >= self.start || min < self.end
        }
    }

    /// Minutes the window is open, once.
    pub fn length_minutes(&self) -> i64 {
        (self.end - self.start).rem_euclid(DAY_MIN)
    }
}

/// Local minutes after midnight for a Unix ms time.
fn minute_of_day(ts: i64, off: OffsetFn) -> i64 {
    (ts + off(ts)).div_euclid(MIN_MS).rem_euclid(DAY_MIN)
}

pub(crate) fn day_number(ts: i64, off: OffsetFn) -> i64 {
    (ts + off(ts)).div_euclid(DAY_MS)
}

/// `YYYY-MM-DD` of a day number (days since 1970-01-01), proleptic Gregorian.
pub fn day_text(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Is `ts` inside the window (local time)?
pub fn in_window(w: &Window, ts: i64, off: OffsetFn) -> bool {
    w.covers(minute_of_day(ts, off))
}

/// The local date of the night a moment belongs to. Inside the window it is the night that is
/// going on (the date the window opened); outside it is the night that comes next.
pub fn night_of(w: &Window, ts: i64, off: OffsetFn) -> String {
    let (today, min) = (day_number(ts, off), minute_of_day(ts, off));
    let day = if w.start < w.end {
        if min >= w.end {
            today + 1
        } else {
            today
        }
    } else if min < w.end {
        // after midnight in a window that opened yesterday
        today - 1
    } else {
        today
    };
    day_text(day)
}

/// `YYYYMMDD` for branch names.
pub fn compact(night: &str) -> String {
    night.replace('-', "")
}

/// The local date of a moment, `YYYY-MM-DD`.
pub fn local_date(ts: i64, off: OffsetFn) -> String {
    day_text(day_number(ts, off))
}

/// When the window next opens at or after `ts` (Unix ms), for "starts at ...".
pub fn next_open(w: &Window, ts: i64, off: OffsetFn) -> i64 {
    let min = minute_of_day(ts, off);
    let ahead = (w.start - min).rem_euclid(DAY_MIN);
    let ahead = if ahead == 0 && !w.covers(min) {
        0
    } else {
        ahead
    };
    ts - ts.rem_euclid(MIN_MS) + ahead * MIN_MS
}

/// Unix ms of a local date (`YYYY-MM-DD`) and time (`HH:MM`) in a zone `off_h` hours east of UTC.
#[cfg(test)]
pub(crate) fn test_ms(day: &str, hm: &str, off_h: i64) -> i64 {
    let n: Vec<i64> = day.split('-').map(|x| x.parse().unwrap()).collect();
    let (y, m, d) = (n[0], n[1], n[2]);
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * DAY_MS + parse_clock(hm).unwrap() * MIN_MS - off_h * 3_600_000
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed zone, UTC+2.
    fn plus2(_: i64) -> i64 {
        2 * 3_600_000
    }

    fn at(day: &str, hm: &str) -> i64 {
        test_ms(day, hm, 2)
    }

    #[test]
    fn parses_and_prints_windows() {
        let w = Window::parse("01:00-07:00").unwrap();
        assert_eq!((w.start, w.end), (60, 420));
        assert_eq!(w.text(), "01:00-07:00");
        assert_eq!(
            Window::parse(" 23:30 - 05:15 ").unwrap().text(),
            "23:30-05:15"
        );
        for bad in [
            "",
            "1-7",
            "25:00-07:00",
            "01:00",
            "01:00-01:00",
            "01:60-02:00",
            "a-b",
        ] {
            assert!(Window::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn window_edges() {
        let w = Window::default();
        let inside = |hm| in_window(&w, at("2026-11-21", hm), &plus2);
        assert!(!inside("00:59"));
        assert!(inside("01:00"));
        assert!(inside("06:59"));
        assert!(!inside("07:00"));
        assert!(!inside("12:00"));
    }

    #[test]
    fn midnight_crossing_window() {
        let w = Window::parse("23:00-05:00").unwrap();
        let inside = |hm| in_window(&w, at("2026-11-21", hm), &plus2);
        assert!(!inside("22:59"));
        assert!(inside("23:00"));
        assert!(inside("00:00"));
        assert!(inside("04:59"));
        assert!(!inside("05:00"));
        assert_eq!(w.length_minutes(), 360);
    }

    #[test]
    fn which_night_a_moment_belongs_to() {
        let w = Window::default();
        let n = |d, hm| night_of(&w, at(d, hm), &plus2);
        // planning after 07:00 means tonight; inside the window it is the current night
        assert_eq!(n("2026-11-21", "09:00"), "2026-11-22");
        assert_eq!(n("2026-11-21", "00:30"), "2026-11-21");
        assert_eq!(n("2026-11-21", "03:00"), "2026-11-21");
        assert_eq!(n("2026-11-21", "06:59"), "2026-11-21");
        assert_eq!(n("2026-11-21", "07:00"), "2026-11-22");
        assert_eq!(n("2026-11-30", "23:00"), "2026-12-01");
        let c = Window::parse("23:00-05:00").unwrap();
        let n = |d, hm| night_of(&c, at(d, hm), &plus2);
        assert_eq!(n("2026-11-21", "23:30"), "2026-11-21");
        assert_eq!(n("2026-11-22", "02:00"), "2026-11-21");
        assert_eq!(n("2026-11-22", "05:00"), "2026-11-22");
        assert_eq!(n("2026-11-22", "12:00"), "2026-11-22");
    }

    #[test]
    fn day_text_handles_leap_years_and_epoch() {
        assert_eq!(day_text(0), "1970-01-01");
        assert_eq!(day_text(19_782), "2024-02-29");
        assert_eq!(day_text(20_413), "2025-11-21");
        assert_eq!(compact("2026-11-21"), "20261121");
    }

    #[test]
    fn next_open_is_the_coming_start() {
        let w = Window::default();
        assert_eq!(
            next_open(&w, at("2026-11-21", "09:00"), &plus2),
            at("2026-11-22", "01:00")
        );
        assert_eq!(
            next_open(&w, at("2026-11-21", "00:10"), &plus2),
            at("2026-11-21", "01:00")
        );
    }

    #[test]
    fn the_machine_offset_is_sane() {
        let off = local_offset(1_700_000_000_000);
        assert!(off.abs() <= 14 * 3_600_000);
    }
}
