//! The cost of every local day in a window, split by project or by model: the data behind the
//! board's daily chart. The top five groups by cost keep their names; the rest fold into "other".

use super::report::{load_cells, project_label, round2, By, Cell, Sum};
use super::{day_of, DAY_MS};
use crate::config::UsageCfg;
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// How many groups keep their own name; the rest fold into [`OTHER`].
const KEPT: usize = 5;
const OTHER: &str = "other";

/// One local day: the usage of each group that had any.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DayPoint {
    pub day: String,
    pub sums: BTreeMap<String, Sum>,
}

/// The days of a window, oldest first, and the groups they are split by.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub by: By,
    /// The kept groups by total cost, biggest first, then "other" when something folded into it.
    pub keys: Vec<String>,
    pub days: Vec<DayPoint>,
}

fn key_of(c: &Cell, by: By) -> String {
    match by {
        By::Model if c.model.is_empty() => "(unknown)".into(),
        By::Model => c.model.clone(),
        _ => project_label(&c.project).unwrap_or_else(|| OTHER.into()),
    }
}

/// The series over the last `days` local days ending today, by project or by model (any other
/// grouping gives the project one).
pub fn build(
    conn: &Connection,
    cfg: &UsageCfg,
    days: i64,
    by: By,
    now: i64,
    offset: &dyn Fn(i64) -> i64,
) -> Result<Series> {
    let days = days.clamp(1, 366);
    let by = if by == By::Model {
        By::Model
    } else {
        By::Project
    };
    let to = day_of(now, offset);
    let from = day_of(now - (days - 1) * DAY_MS, offset);
    let cells = load_cells(conn, &from, &to)?;
    let mut totals: BTreeMap<String, Sum> = BTreeMap::new();
    for c in &cells {
        totals
            .entry(key_of(c, by))
            .or_default()
            .add(c.t, &c.model, cfg);
    }
    let mut ranked: Vec<(String, Sum)> = totals.into_iter().filter(|(k, _)| k != OTHER).collect();
    ranked.sort_by(|a, b| {
        b.1.cost
            .total_cmp(&a.1.cost)
            .then(b.1.total().cmp(&a.1.total()))
            .then(a.0.cmp(&b.0))
    });
    let mut keys: Vec<String> = ranked.into_iter().take(KEPT).map(|(k, _)| k).collect();
    let mut points: Vec<DayPoint> = (0..days)
        .map(|i| DayPoint {
            day: day_of(now - (days - 1 - i) * DAY_MS, offset),
            sums: BTreeMap::new(),
        })
        .collect();
    let mut folded = false;
    for c in &cells {
        let Some(p) = points.iter_mut().find(|p| p.day == c.day) else {
            continue;
        };
        let k = key_of(c, by);
        let k = if keys.contains(&k) {
            k
        } else {
            folded = true;
            OTHER.to_string()
        };
        p.sums.entry(k).or_default().add(c.t, &c.model, cfg);
    }
    if folded {
        keys.push(OTHER.into());
    }
    Ok(Series {
        by,
        keys,
        days: points,
    })
}

impl Series {
    /// Each day with everything the groups used on it.
    pub fn day_totals(&self) -> Vec<(String, Sum)> {
        self.days
            .iter()
            .map(|p| {
                let mut all = Sum::default();
                for s in p.sums.values() {
                    for (a, b) in all.t.iter_mut().zip(s.t) {
                        *a += b;
                    }
                    all.cost += s.cost;
                    all.unpriced += s.unpriced;
                }
                (p.day.clone(), all)
            })
            .collect()
    }

    /// The JSON of the API's `series` field. A group with no priced token on a day has no `cost`
    /// entry there; its tokens still count.
    pub fn json(&self) -> Value {
        let days: Vec<Value> = self
            .days
            .iter()
            .map(|p| {
                let mut cost = serde_json::Map::new();
                let mut tokens = serde_json::Map::new();
                for (k, s) in &p.sums {
                    if !s.cost_unknown() {
                        cost.insert(k.clone(), json!(round2(s.cost)));
                    }
                    tokens.insert(k.clone(), json!(s.total()));
                }
                json!({"day": p.day, "cost": cost, "tokens": tokens})
            })
            .collect();
        json!({"by": self.by.name(), "keys": self.keys, "days": days})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const NOW: i64 = 20_100 * DAY_MS + 12 * 3_600_000;

    fn put(db: &Db, sid: &str, day: i64, model: &str, input: i64) {
        db.conn
            .execute(
                "INSERT INTO usage_day VALUES(?1,?2,?3,?4,0,0,0)",
                rusqlite::params![sid, crate::worked::summary::day_name(day), model, input],
            )
            .unwrap();
    }

    fn session(db: &Db, id: &str, repo: &str) {
        db.conn
            .execute(
                "INSERT INTO session(id, agent, repo_root) VALUES(?1,'claude',?2)",
                rusqlite::params![id, repo],
            )
            .unwrap();
    }

    fn series(db: &Db, days: i64, by: By) -> Series {
        build(&db.conn, &UsageCfg::default(), days, by, NOW, &|_| 0).unwrap()
    }

    #[test]
    fn an_empty_window_still_has_every_day_and_no_keys() {
        let db = Db::open_memory().unwrap();
        let s = series(&db, 14, By::Project);
        assert!(s.keys.is_empty());
        assert_eq!(s.days.len(), 14);
        assert_eq!(
            s.days[13].day,
            crate::worked::summary::day_name(NOW / DAY_MS)
        );
        assert!(s.days.iter().all(|d| d.sums.is_empty()));
        let v = s.json();
        assert_eq!(v["by"], "project");
        assert_eq!(v["days"][0]["cost"], json!({}));
    }

    #[test]
    fn the_top_five_projects_keep_their_names_and_the_rest_fold_into_other() {
        let db = Db::open_memory().unwrap();
        let today = NOW / DAY_MS;
        for i in 0..7 {
            session(&db, &format!("s{i}"), &format!("/p/proj{i}"));
            put(
                &db,
                &format!("s{i}"),
                today,
                "claude-opus-5-5",
                1_000_000 * (10 - i),
            );
        }
        let s = series(&db, 14, By::Project);
        assert_eq!(
            s.keys,
            ["proj0", "proj1", "proj2", "proj3", "proj4", "other"]
        );
        let last = &s.days[13];
        // proj5 (5M) and proj6 (4M) fold: $20 + $16 at $4 per million input tokens
        assert_eq!(last.sums["other"].cost, 36.0);
        assert_eq!(s.json()["days"][13]["cost"]["proj0"], 40.0);
    }

    #[test]
    fn a_window_of_five_or_fewer_groups_has_no_other() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "/p/one");
        put(&db, "a", NOW / DAY_MS - 2, "claude-opus-5-5", 1_000_000);
        let s = series(&db, 14, By::Project);
        assert_eq!(s.keys, ["one"]);
        assert_eq!(s.days[11].sums["one"].cost, 4.0);
        assert!(s.days[13].sums.is_empty());
    }

    #[test]
    fn day_totals_add_the_groups_of_each_day() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "/p/one");
        session(&db, "b", "/p/two");
        let today = NOW / DAY_MS;
        put(&db, "a", today, "claude-opus-5-5", 1_000_000);
        put(&db, "b", today, "claude-opus-5-5", 500_000);
        let t = series(&db, 3, By::Project).day_totals();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].1.total(), 0);
        assert_eq!(t[2].1.cost, 6.0);
        assert_eq!(t[2].1.total(), 1_500_000);
    }

    #[test]
    fn days_outside_the_window_are_left_out() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "/p/one");
        put(&db, "a", NOW / DAY_MS - 14, "claude-opus-5-5", 1_000_000);
        let s = series(&db, 14, By::Project);
        assert!(s.keys.is_empty());
    }

    #[test]
    fn by_model_has_an_unknown_model_with_tokens_and_no_cost() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "/p/one");
        let today = NOW / DAY_MS;
        put(&db, "a", today, "claude-opus-5-5", 1_000_000);
        put(&db, "a", today, "claude-from-the-future", 10);
        let s = series(&db, 7, By::Model);
        assert_eq!(s.keys, ["claude-opus-5-5", "claude-from-the-future"]);
        let v = s.json();
        let d = &v["days"][6];
        assert_eq!(d["cost"]["claude-opus-5-5"], 4.0);
        assert!(d["cost"].get("claude-from-the-future").is_none());
        assert_eq!(d["tokens"]["claude-from-the-future"], 10);
    }

    #[test]
    fn temp_folders_and_folder_less_sessions_count_as_other() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "/p/one");
        session(&db, "b", "");
        put(&db, "a", NOW / DAY_MS, "claude-opus-5-5", 1_000_000);
        put(&db, "b", NOW / DAY_MS, "claude-opus-5-5", 500_000);
        let s = series(&db, 7, By::Project);
        assert_eq!(s.keys, ["one", "other"]);
        assert_eq!(s.days[6].sums["other"].cost, 2.0);
    }
}
