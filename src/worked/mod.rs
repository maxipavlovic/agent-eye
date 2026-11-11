//! The days a session was worked on. An adapter reports the time of every message of a session
//! (`Delta::events`); the core adds them into UTC hours here, stores the hours (`store`), and turns
//! them into local days when it answers (`summary`). Sessions read before eye kept hours get theirs
//! from `backfill`, which reads timestamps only.

pub mod backfill;
pub mod store;
pub mod summary;

use std::collections::BTreeMap;

/// A gap between two messages longer than this is time away, not time worked.
pub const AWAY_MS: i64 = 15 * 60 * 1000;
const HOUR_MS: i64 = 3_600_000;

/// One message of a session: when it happened, and whether the user typed it (a prompt) or it was
/// a reply or a tool result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkEvent {
    pub ts: i64,
    pub prompt: bool,
}

/// What happened in one UTC hour of a session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hour {
    pub prompts: i64,
    pub events: i64,
    pub active_ms: i64,
}

/// The UTC hour a time falls in, as hours since the epoch.
pub fn hour_of(ts: i64) -> i64 {
    ts.div_euclid(HOUR_MS)
}

/// The start of an hour, in Unix ms.
pub fn hour_start(hour: i64) -> i64 {
    hour * HOUR_MS
}

/// Add `events` into hours. `last` is the time of the last message already counted: a message at
/// or before it was counted before and is left out, and the gap from it to the first new message
/// counts like any other. Returns the hours added to and the time of the last message counted
/// (`last` again when nothing new came).
pub fn bucket(last: Option<i64>, events: &[WorkEvent]) -> (BTreeMap<i64, Hour>, Option<i64>) {
    let mut sorted: Vec<WorkEvent> = events
        .iter()
        .copied()
        .filter(|e| last.is_none_or(|l| e.ts > l))
        .collect();
    sorted.sort_by_key(|e| e.ts);
    let mut hours: BTreeMap<i64, Hour> = BTreeMap::new();
    let mut prev = last;
    for e in &sorted {
        let h = hours.entry(hour_of(e.ts)).or_default();
        h.events += 1;
        h.prompts += i64::from(e.prompt);
        if let Some(p) = prev.filter(|p| e.ts - p <= AWAY_MS) {
            add_gap(&mut hours, p, e.ts);
        }
        prev = Some(e.ts);
    }
    (hours, prev)
}

/// Active time from `from` to `to`, split at the hour boundaries it crosses.
fn add_gap(hours: &mut BTreeMap<i64, Hour>, from: i64, to: i64) {
    let mut t = from;
    while t < to {
        let h = hour_of(t);
        let end = hour_start(h + 1).min(to);
        hours.entry(h).or_default().active_ms += end - t;
        t = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn ev(min: i64, prompt: bool) -> WorkEvent {
        WorkEvent {
            ts: 1_000 * HOUR_MS + min * MIN,
            prompt,
        }
    }

    #[test]
    fn gaps_up_to_fifteen_minutes_count_as_work() {
        let (hours, last) = bucket(None, &[ev(0, true), ev(10, false), ev(25, false)]);
        let h = hours[&1_000];
        assert_eq!(h.active_ms, 25 * MIN);
        assert_eq!((h.prompts, h.events), (1, 3));
        assert_eq!(last, Some(ev(25, false).ts));
    }

    #[test]
    fn a_longer_gap_counts_as_away() {
        let (hours, _) = bucket(None, &[ev(0, true), ev(16, false), ev(20, false)]);
        assert_eq!(hours[&1_000].active_ms, 4 * MIN);
    }

    #[test]
    fn a_gap_over_an_hour_boundary_is_split_between_the_hours() {
        let (hours, _) = bucket(None, &[ev(55, true), ev(65, false)]);
        assert_eq!(hours[&1_000].active_ms, 5 * MIN);
        assert_eq!(hours[&1_001].active_ms, 5 * MIN);
        assert_eq!(hours[&1_001].events, 1);
    }

    #[test]
    fn the_gap_across_two_reads_counts_once() {
        let (first, last) = bucket(None, &[ev(0, true), ev(5, false)]);
        let (second, last) = bucket(last, &[ev(9, false), ev(12, true)]);
        let total = first[&1_000].active_ms + second[&1_000].active_ms;
        assert_eq!(total, 12 * MIN);
        assert_eq!(last, Some(ev(12, true).ts));
    }

    #[test]
    fn messages_already_counted_are_left_out() {
        let (hours, last) = bucket(Some(ev(5, false).ts), &[ev(0, true), ev(5, false)]);
        assert!(hours.is_empty());
        assert_eq!(last, Some(ev(5, false).ts));
    }

    #[test]
    fn messages_out_of_order_are_sorted_first() {
        let (hours, _) = bucket(None, &[ev(10, false), ev(0, true)]);
        assert_eq!(hours[&1_000].active_ms, 10 * MIN);
    }

    #[test]
    fn no_messages_give_no_hours() {
        let (hours, last) = bucket(None, &[]);
        assert!(hours.is_empty());
        assert_eq!(last, None);
    }
}
