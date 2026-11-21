//! The plan limits an agent reports, kept as the newest seen per agent (`limits:<agent>` in `meta`)
//! with the time they were seen, and shown with their age. Codex rollouts carry them in
//! `token_count` events; Claude Code hands them to the status line command (`eye statusline`).

use super::{LimitWindow, PlanLimits};
use anyhow::Result;
use chrono::{Local, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};

fn key(agent: &str) -> String {
    format!("limits:{agent}")
}

/// Keep `l` as the limits of `agent` unless what is stored was seen later. Stored as JSON.
pub fn record(conn: &Connection, agent: &str, l: &PlanLimits) -> Result<()> {
    if let Some(old) = load(conn, agent)? {
        if old.seen_at > l.seen_at {
            return Ok(());
        }
    }
    conn.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key(agent), serde_json::to_string(l)?],
    )?;
    Ok(())
}

/// The limits stored for `agent`.
pub fn load(conn: &Connection, agent: &str) -> Result<Option<PlanLimits>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key=?1", [key(agent)], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| serde_json::from_str(&v).ok()))
}

/// What a window covers, in words: `5-hour`, `weekly`, `monthly`, or the agent's own name for it.
pub fn label(w: &LimitWindow) -> String {
    match w.window_minutes {
        Some(300) => "5-hour".into(),
        Some(10_080) => "weekly".into(),
        Some(43_200) | Some(44_640) => "monthly".into(),
        Some(m) if m % 1_440 == 0 => format!("{}-day", m / 1_440),
        Some(m) if m % 60 == 0 => format!("{}-hour", m / 60),
        Some(m) => format!("{m}-minute"),
        None => match w.name.as_str() {
            "five_hour" => "5-hour".into(),
            "seven_day" => "weekly".into(),
            "spend_limit" => "spend".into(),
            other => other.replace('_', " "),
        },
    }
}

/// `Mon 09:00` in local time.
pub fn reset_text(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .earliest()
        .map_or_else(String::new, |d| d.format("%a %H:%M").to_string())
}

fn percent(p: f64) -> String {
    if p.fract().abs() < 0.05 {
        format!("{p:.0}%")
    } else {
        format!("{p:.1}%")
    }
}

/// "just now", "12 min ago", "3 h ago", "2 d ago".
fn age(now: i64, ms: i64) -> String {
    let s = (now - ms).max(0) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86400),
    }
}

/// One line per window: `Codex: 23% of the weekly limit, resets Mon 09:00 (seen 12 min ago)`.
pub fn lines(agent_name: &str, l: &PlanLimits, now: i64) -> Vec<String> {
    let seen = age(now, l.seen_at);
    l.windows
        .iter()
        .map(|w| {
            let reset = match w.resets_at {
                Some(r) if r <= now => " (window has reset since)".to_string(),
                Some(r) => format!(", resets {}", reset_text(r)),
                None => String::new(),
            };
            format!(
                "{agent_name}: {} of the {} limit{reset} (seen {seen})",
                percent(w.used_percent),
                label(w)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn limits(seen_at: i64, pct: f64) -> PlanLimits {
        PlanLimits {
            seen_at,
            plan: None,
            windows: vec![LimitWindow {
                name: "secondary".into(),
                used_percent: pct,
                window_minutes: Some(10_080),
                resets_at: Some(seen_at + 3_600_000),
            }],
        }
    }

    #[test]
    fn the_newest_limits_win_whatever_order_they_are_read_in() {
        let db = Db::open_memory().unwrap();
        record(&db.conn, "codex", &limits(2_000, 40.0)).unwrap();
        record(&db.conn, "codex", &limits(1_000, 10.0)).unwrap();
        assert_eq!(
            load(&db.conn, "codex").unwrap().unwrap().windows[0].used_percent,
            40.0
        );
        record(&db.conn, "codex", &limits(3_000, 50.0)).unwrap();
        assert_eq!(load(&db.conn, "codex").unwrap().unwrap().seen_at, 3_000);
        assert!(load(&db.conn, "claude").unwrap().is_none());
    }

    #[test]
    fn a_window_reads_as_a_sentence_with_its_age() {
        let l = limits(1_000_000, 23.0);
        let out = lines("Codex", &l, 1_000_000 + 12 * 60_000);
        assert!(
            out[0].starts_with("Codex: 23% of the weekly limit, resets "),
            "{out:?}"
        );
        assert!(out[0].ends_with("(seen 12 min ago)"), "{out:?}");
        let gone = lines("Codex", &l, 1_000_000 + 2 * 3_600_000);
        assert!(gone[0].contains("window has reset since"), "{gone:?}");
    }

    #[test]
    fn windows_are_named_by_their_length() {
        let w = |m| LimitWindow {
            window_minutes: Some(m),
            ..Default::default()
        };
        assert_eq!(label(&w(300)), "5-hour");
        assert_eq!(label(&w(10_080)), "weekly");
        assert_eq!(label(&w(43_200)), "monthly");
        assert_eq!(label(&w(2_880)), "2-day");
    }
}
