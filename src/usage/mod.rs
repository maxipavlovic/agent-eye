//! What a session cost, by day and model, and how much of the plan is used. An adapter reports the
//! token use of every message of a session (`Delta::usage_msgs`); the core adds them into local days
//! per model (`store`), prices them (`prices`) and answers questions about them (`report`). Sessions
//! read before eye kept usage by day get theirs from `backfill`, which never touches a read cursor.
//! Plan limits an agent reports are kept as the newest seen (`limits`); the Claude Code status line
//! feeds them through `eye statusline` (`statusline`).
//!
//! Costs are an API-equivalent estimate at list prices, not a bill: a subscription is a flat fee.

pub mod backfill;
pub mod limits;
pub mod prices;
pub mod report;
pub mod statusline;
pub mod store;

use std::collections::BTreeMap;

/// The token use of one assistant message: when, on which model, and the four counts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageMsg {
    pub ts: i64,
    pub model: String,
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
}

impl UsageMsg {
    pub fn is_empty(&self) -> bool {
        self.input + self.output + self.cache_write + self.cache_read == 0
    }
}

/// One window of a plan limit (the 5-hour or the weekly one).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct LimitWindow {
    /// The agent's name for it (`primary`, `five_hour`, `seven_day`, `spend_limit`).
    pub name: String,
    /// 0 to 100, or above once the limit is passed.
    pub used_percent: f64,
    pub window_minutes: Option<i64>,
    /// When the window resets, Unix ms.
    pub resets_at: Option<i64>,
}

/// The plan limits an agent reported, and when it said so.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct PlanLimits {
    pub seen_at: i64,
    pub plan: Option<String>,
    pub windows: Vec<LimitWindow>,
}

const DAY_MS: i64 = 86_400_000;

/// `YYYY-MM-DD` of the local day a time falls on, with the offset of the time zone at that time.
pub fn day_of(ts: i64, offset: &dyn Fn(i64) -> i64) -> String {
    crate::worked::summary::day_name((ts + offset(ts)).div_euclid(DAY_MS))
}

/// Add messages into `(local day, model)` buckets of `[input, output, cache write, cache read]`.
pub fn by_day(
    msgs: &[UsageMsg],
    offset: &dyn Fn(i64) -> i64,
) -> BTreeMap<(String, String), [i64; 4]> {
    let mut out: BTreeMap<(String, String), [i64; 4]> = BTreeMap::new();
    for m in msgs.iter().filter(|m| !m.is_empty()) {
        let e = out
            .entry((day_of(m.ts, offset), m.model.clone()))
            .or_default();
        e[0] += m.input;
        e[1] += m.output;
        e[2] += m.cache_write;
        e[3] += m.cache_read;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(ts: i64, model: &str, i: i64, o: i64) -> UsageMsg {
        UsageMsg {
            ts,
            model: model.into(),
            input: i,
            output: o,
            ..Default::default()
        }
    }

    #[test]
    fn messages_add_up_per_local_day_and_model() {
        let utc = |_: i64| 0;
        let d0 = 20_000 * DAY_MS;
        let msgs = [
            msg(d0 + 1, "a", 10, 1),
            msg(d0 + 2, "a", 5, 1),
            msg(d0 + 3, "b", 7, 2),
            msg(d0 + DAY_MS, "a", 1, 1),
            msg(d0 + 4, "a", 0, 0),
        ];
        let m = by_day(&msgs, &utc);
        assert_eq!(m.len(), 3);
        let day0 = day_of(d0, &utc);
        assert_eq!(m[&(day0.clone(), "a".into())], [15, 2, 0, 0]);
        assert_eq!(m[&(day0, "b".into())], [7, 2, 0, 0]);
    }

    #[test]
    fn the_local_offset_moves_a_late_message_to_the_next_day() {
        let plus2 = |_: i64| 2 * 3_600_000;
        let ts = 20_000 * DAY_MS + 23 * 3_600_000;
        assert_eq!(day_of(ts, &plus2), day_of(ts + DAY_MS / 12 * 2, &|_| 0));
        assert_ne!(day_of(ts, &plus2), day_of(ts, &|_| 0));
    }
}
