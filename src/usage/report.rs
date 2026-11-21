//! Usage over a window of local days, grouped four ways, with the cost estimate, the four-week
//! trend and the plan limits. The CLI (`eye usage`), the board (`GET /api/usage`) and the session
//! panel all read it from here.

use super::{day_of, limits, prices, PlanLimits};
use crate::agents::Agents;
use crate::config::UsageCfg;
use crate::util::{fmt_tokens, home_rel};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const DAY_MS: i64 = 86_400_000;
/// The trend covers this many weeks, ending today.
pub const TREND_WEEKS: i64 = 4;

/// What rows are grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    Project,
    Agent,
    Model,
    Session,
}

impl By {
    pub fn parse(s: &str) -> Option<By> {
        match s {
            "project" => Some(By::Project),
            "agent" => Some(By::Agent),
            "model" => Some(By::Model),
            "session" => Some(By::Session),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            By::Project => "project",
            By::Agent => "agent",
            By::Model => "model",
            By::Session => "session",
        }
    }
}

/// Tokens and the estimated cost of a group of messages.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sum {
    /// `[input, output, cache write, cache read]`.
    pub t: [i64; 4],
    /// The cost of the tokens that have a price, USD.
    pub cost: f64,
    /// Tokens whose model has no price: counted above, not in `cost`.
    pub unpriced: i64,
}

impl Sum {
    fn add(&mut self, t: [i64; 4], model: &str, cfg: &UsageCfg) {
        for (a, b) in self.t.iter_mut().zip(t) {
            *a += b;
        }
        match prices::cost_of(model, t, cfg) {
            Some(c) => self.cost += c,
            None => self.unpriced += t.iter().sum::<i64>(),
        }
    }
    pub fn total(&self) -> i64 {
        self.t.iter().sum()
    }
    /// No token has a price: the cost is unknown, not zero.
    pub fn cost_unknown(&self) -> bool {
        self.unpriced > 0 && self.unpriced >= self.total()
    }
    /// `$12.40`, `$12.40 + unpriced`, or `unknown`.
    pub fn cost_text(&self) -> String {
        if self.total() == 0 {
            "$0.00".into()
        } else if self.cost_unknown() {
            "unknown".into()
        } else if self.unpriced > 0 {
            format!("${:.2} + unpriced", self.cost)
        } else {
            format!("${:.2}", self.cost)
        }
    }
    fn json(&self) -> Value {
        json!({
            "input": self.t[0], "output": self.t[1], "cache_write": self.t[2], "cache_read": self.t[3],
            "total": self.total(),
            "cost_usd": if self.cost_unknown() { Value::Null } else { json!(round2(self.cost)) },
            "unpriced_tokens": self.unpriced,
        })
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// One group of rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub name: String,
    /// For a session row: its id and agent.
    pub id: Option<String>,
    pub agent: Option<String>,
    pub sum: Sum,
}

impl Row {
    fn json(&self) -> Value {
        let mut v = self.sum.json();
        v["name"] = json!(self.name);
        if let Some(id) = &self.id {
            v["id"] = json!(id);
        }
        if let Some(a) = &self.agent {
            v["agent"] = json!(a);
        }
        v
    }
}

/// One of the weeks of the trend.
#[derive(Debug, Clone, PartialEq)]
pub struct Week {
    pub from: String,
    pub to: String,
    pub sum: Sum,
}

/// The plan limits of one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentLimits {
    pub agent: String,
    pub name: String,
    pub limits: PlanLimits,
}

/// Everything the CLI, the API and the board show.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub days: i64,
    pub from: String,
    pub to: String,
    pub by: Option<By>,
    pub rows: Vec<Row>,
    pub totals: Sum,
    pub by_project: Vec<Row>,
    pub by_agent: Vec<Row>,
    pub by_model: Vec<Row>,
    pub top_sessions: Vec<Row>,
    pub weeks: Vec<Week>,
    pub limits: Vec<AgentLimits>,
    /// Sessions whose usage is not counted yet (the background pass is still at it).
    pub backfill_left: usize,
}

struct Cell {
    sid: String,
    day: String,
    model: String,
    agent: String,
    project: String,
    title: String,
    t: [i64; 4],
}

fn load_cells(conn: &Connection, from: &str, to: &str) -> Result<Vec<Cell>> {
    let mut st = conn.prepare(
        "SELECT u.session_id, u.day, u.model, COALESCE(s.agent,'claude'),
                COALESCE(s.repo_root, s.cwd, s.project_dir, ''), COALESCE(s.title, s.first_prompt, ''),
                u.tok_in, u.tok_out, u.tok_cw, u.tok_cr
         FROM usage_day u LEFT JOIN session s ON s.id=u.session_id
         WHERE u.day >= ?1 AND u.day <= ?2",
    )?;
    let rows = st
        .query_map([from, to], |r| {
            Ok(Cell {
                sid: r.get(0)?,
                day: r.get(1)?,
                model: r.get(2)?,
                agent: r.get(3)?,
                project: r.get(4)?,
                title: r.get(5)?,
                t: [r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?],
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn project_name(p: &str) -> String {
    if p.is_empty() {
        "(unknown)".into()
    } else {
        home_rel(p)
    }
}

fn group(cells: &[&Cell], by: By, cfg: &UsageCfg) -> Vec<Row> {
    let mut map: BTreeMap<String, Row> = BTreeMap::new();
    for c in cells {
        let (key, name) = match by {
            By::Project => (project_name(&c.project), project_name(&c.project)),
            By::Agent => (c.agent.clone(), c.agent.clone()),
            By::Model => {
                let m = if c.model.is_empty() {
                    "(unknown)"
                } else {
                    &c.model
                };
                (m.to_string(), m.to_string())
            }
            By::Session => (c.sid.clone(), c.title.clone()),
        };
        let row = map.entry(key).or_insert_with(|| Row {
            name,
            id: (by == By::Session).then(|| c.sid.clone()),
            agent: (by == By::Session).then(|| c.agent.clone()),
            ..Default::default()
        });
        row.sum.add(c.t, &c.model, cfg);
    }
    let mut rows: Vec<Row> = map.into_values().collect();
    rows.sort_by(|a, b| {
        b.sum
            .cost
            .total_cmp(&a.sum.cost)
            .then(b.sum.total().cmp(&a.sum.total()))
            .then(a.name.cmp(&b.name))
    });
    rows
}

/// Build the report for the last `days` local days ending today (`now`). The offset is the time
/// zone's (a function so tests can fix it).
pub fn build(
    conn: &Connection,
    cfg: &UsageCfg,
    agents: &Agents,
    days: i64,
    by: By,
    now: i64,
    offset: &dyn Fn(i64) -> i64,
) -> Result<Report> {
    let days = days.clamp(1, 366);
    let to = day_of(now, offset);
    let from = day_of(now - (days - 1) * DAY_MS, offset);
    let trend_from = day_of(now - (TREND_WEEKS * 7 - 1) * DAY_MS, offset);
    let load_from = if trend_from < from {
        &trend_from
    } else {
        &from
    };
    let cells = load_cells(conn, load_from, &to)?;
    let window: Vec<&Cell> = cells
        .iter()
        .filter(|c| c.day.as_str() >= from.as_str())
        .collect();
    let mut r = Report {
        days,
        from,
        to: to.clone(),
        by: Some(by),
        ..Default::default()
    };
    for c in &window {
        r.totals.add(c.t, &c.model, cfg);
    }
    r.rows = group(&window, by, cfg);
    r.by_project = group(&window, By::Project, cfg);
    r.by_agent = group(&window, By::Agent, cfg);
    r.by_model = group(&window, By::Model, cfg);
    r.top_sessions = group(&window, By::Session, cfg);
    r.top_sessions.truncate(10);
    for w in (0..TREND_WEEKS).rev() {
        let wto = day_of(now - w * 7 * DAY_MS, offset);
        let wfrom = day_of(now - (w * 7 + 6) * DAY_MS, offset);
        let mut sum = Sum::default();
        for c in cells
            .iter()
            .filter(|c| c.day.as_str() >= wfrom.as_str() && c.day.as_str() <= wto.as_str())
        {
            sum.add(c.t, &c.model, cfg);
        }
        r.weeks.push(Week {
            from: wfrom,
            to: wto,
            sum,
        });
    }
    for a in agents.iter() {
        if let Some(l) = limits::load(conn, a.id())? {
            r.limits.push(AgentLimits {
                agent: a.id().to_string(),
                name: a.short_name().to_string(),
                limits: l,
            });
        }
    }
    r.backfill_left = conn.query_row(
        "SELECT count(*) FROM session s WHERE transcript_state IN ('present','archived')
         AND agent IN (SELECT value FROM json_each(?1))
         AND NOT EXISTS (SELECT 1 FROM usage_state w WHERE w.session_id=s.id)",
        [json!(agents.iter().map(|a| a.id()).collect::<Vec<_>>()).to_string()],
        |x| x.get::<_, i64>(0),
    )? as usize;
    Ok(r)
}

impl Report {
    /// The JSON the API and `eye usage --json` give.
    pub fn json(&self, now: i64) -> Value {
        let rows = |v: &[Row]| v.iter().map(Row::json).collect::<Vec<_>>();
        json!({
            "estimate": prices::LABEL,
            "days": self.days, "from": self.from, "to": self.to,
            "by": self.by.map(By::name),
            "rows": rows(&self.rows),
            "totals": self.totals.json(),
            "by_project": rows(&self.by_project),
            "by_agent": rows(&self.by_agent),
            "by_model": rows(&self.by_model),
            "top_sessions": rows(&self.top_sessions),
            "weeks": self.weeks.iter().map(|w| {
                let mut v = w.sum.json();
                v["from"] = json!(w.from);
                v["to"] = json!(w.to);
                v
            }).collect::<Vec<_>>(),
            "limits": self.limits.iter().map(|l| json!({
                "agent": l.agent, "name": l.name, "seen_at": l.limits.seen_at, "plan": l.limits.plan,
                "windows": l.limits.windows.iter().map(|w| json!({
                    "label": limits::label(w), "used_percent": w.used_percent,
                    "window_minutes": w.window_minutes, "resets_at": w.resets_at,
                    "reset": w.resets_at.is_some_and(|r| r <= now),
                })).collect::<Vec<_>>(),
                "lines": limits::lines(&l.name, &l.limits, now),
            })).collect::<Vec<_>>(),
            "backfill_left": self.backfill_left,
        })
    }

    /// The limit lines for the top of the terminal output.
    pub fn limit_lines(&self, now: i64) -> Vec<String> {
        self.limits
            .iter()
            .flat_map(|l| limits::lines(&l.name, &l.limits, now))
            .collect()
    }
}

/// The usage of one session over its whole life.
pub fn of_session(conn: &Connection, cfg: &UsageCfg, sid: &str) -> Result<Option<Sum>> {
    let mut st = conn.prepare(
        "SELECT model, sum(tok_in), sum(tok_out), sum(tok_cw), sum(tok_cr) FROM usage_day WHERE session_id=?1 GROUP BY model",
    )?;
    let mut sum = Sum::default();
    let mut any = false;
    let rows = st.query_map([sid], |r| {
        Ok((
            r.get::<_, String>(0)?,
            [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
        ))
    })?;
    for row in rows {
        let (m, t) = row?;
        sum.add(t, &m, cfg);
        any = true;
    }
    Ok(any.then_some(sum))
}

/// `4.2M tokens (in 1.2k, out 90k, cache 4.1M), $3.40 estimated`.
pub fn session_line(s: &Sum) -> String {
    format!(
        "{} tokens (in {}, out {}, cache {}), {} estimated",
        fmt_tokens(s.total()),
        fmt_tokens(s.t[0]),
        fmt_tokens(s.t[1]),
        fmt_tokens(s.t[2] + s.t[3]),
        if s.cost_unknown() {
            "cost unknown".to_string()
        } else {
            s.cost_text()
        }
    )
}

/// The session's usage as JSON for the card.
pub fn session_json(s: &Sum) -> Value {
    let mut v = s.json();
    v["estimate"] = json!(prices::LABEL);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const NOW: i64 = 20_100 * DAY_MS + 12 * 3_600_000;

    fn put(db: &Db, sid: &str, day: i64, model: &str, t: [i64; 4]) {
        let d = crate::worked::summary::day_name(day);
        db.conn
            .execute(
                "INSERT INTO usage_day VALUES(?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![sid, d, model, t[0], t[1], t[2], t[3]],
            )
            .unwrap();
    }

    fn session(db: &Db, id: &str, agent: &str, repo: &str) {
        db.conn
            .execute(
                "INSERT INTO session(id, agent, repo_root, title) VALUES(?1,?2,?3,?4)",
                rusqlite::params![id, agent, repo, format!("title {id}")],
            )
            .unwrap();
    }

    fn report(db: &Db, days: i64, by: By) -> Report {
        build(
            &db.conn,
            &UsageCfg::default(),
            &Agents::default(),
            days,
            by,
            NOW,
            &|_| 0,
        )
        .unwrap()
    }

    #[test]
    fn the_window_counts_the_last_days_and_groups_by_project_and_agent() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "claude", "/p/one");
        session(&db, "b", "codex", "/p/two");
        let today = NOW / DAY_MS;
        put(&db, "a", today, "claude-opus-5-5", [1_000_000, 0, 0, 0]);
        put(&db, "a", today - 3, "claude-opus-5-5", [1_000_000, 0, 0, 0]);
        put(
            &db,
            "a",
            today - 10,
            "claude-opus-5-5",
            [1_000_000, 0, 0, 0],
        );
        put(&db, "b", today, "gpt-6-luna", [2_000_000, 0, 0, 0]);
        let r = report(&db, 7, By::Project);
        assert_eq!(r.totals.t[0], 4_000_000);
        assert_eq!(r.rows[0].name, "/p/one");
        assert!((r.rows[0].sum.cost - 8.0).abs() < 1e-9);
        assert_eq!(r.by_agent.len(), 2);
        assert_eq!(report(&db, 30, By::Agent).totals.t[0], 5_000_000);
    }

    #[test]
    fn an_unknown_model_shows_tokens_and_no_cost() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "claude", "/p");
        let today = NOW / DAY_MS;
        put(&db, "a", today, "claude-from-the-future", [100, 50, 0, 0]);
        let r = report(&db, 7, By::Model);
        assert_eq!(r.totals.total(), 150);
        assert!(r.totals.cost_unknown());
        assert_eq!(r.totals.cost_text(), "unknown");
        assert_eq!(r.json(NOW)["totals"]["cost_usd"], Value::Null);
        put(&db, "a", today, "claude-opus-5-5", [1_000_000, 0, 0, 0]);
        let r = report(&db, 7, By::Model);
        assert!(
            r.totals.cost_text().ends_with("+ unpriced"),
            "{}",
            r.totals.cost_text()
        );
        assert_eq!(r.rows[0].name, "claude-opus-5-5");
    }

    #[test]
    fn the_trend_has_four_weeks_ending_today() {
        let db = Db::open_memory().unwrap();
        session(&db, "a", "claude", "/p");
        let today = NOW / DAY_MS;
        put(&db, "a", today, "claude-sonnet-5-5", [10, 0, 0, 0]);
        put(&db, "a", today - 8, "claude-sonnet-5-5", [20, 0, 0, 0]);
        put(&db, "a", today - 27, "claude-sonnet-5-5", [40, 0, 0, 0]);
        put(&db, "a", today - 28, "claude-sonnet-5-5", [80, 0, 0, 0]);
        let r = report(&db, 1, By::Project);
        let totals: Vec<i64> = r.weeks.iter().map(|w| w.sum.total()).collect();
        assert_eq!(totals, [40, 0, 20, 10]);
        assert_eq!(r.weeks[3].to, crate::worked::summary::day_name(today));
        assert_eq!(r.totals.total(), 10);
    }

    #[test]
    fn a_session_line_says_tokens_and_the_estimate() {
        let db = Db::open_memory().unwrap();
        let today = NOW / DAY_MS;
        put(
            &db,
            "a",
            today,
            "claude-opus-5-5",
            [1_000_000, 100_000, 0, 2_000_000],
        );
        let s = of_session(&db.conn, &UsageCfg::default(), "a")
            .unwrap()
            .unwrap();
        let line = session_line(&s);
        assert!(
            line.starts_with("3.1M tokens (in 1M, out 100k, cache 2M), $"),
            "{line}"
        );
        assert!(of_session(&db.conn, &UsageCfg::default(), "none")
            .unwrap()
            .is_none());
    }

    #[test]
    fn an_empty_index_gives_an_empty_report() {
        let db = Db::open_memory().unwrap();
        let r = report(&db, 7, By::Project);
        assert!(r.rows.is_empty());
        assert_eq!(r.totals.total(), 0);
        assert_eq!(r.json(NOW)["rows"], json!([]));
    }
}
