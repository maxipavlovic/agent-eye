//! The token use of a Codex rollout by message, and the plan limits it reports. A rollout's
//! `token_count` lines carry the thread's running total; what one step used is the growth since
//! the line before. The model is the one the last `turn_context` named.

use super::rollout::{read_line, Event, Usage};
use crate::agents::append_only::for_each_line;
use crate::usage::{LimitWindow, PlanLimits, UsageMsg};
use crate::util::iso_to_ms;
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Follows a rollout line by line and yields the usage of each `token_count` step.
#[derive(Debug, Default)]
pub struct Steps {
    prev: Usage,
    pub model: Option<String>,
}

impl Steps {
    /// Start from where an earlier read ended: its total and its model.
    pub fn resume(total: Option<Usage>, model: Option<String>) -> Steps {
        Steps {
            prev: total.unwrap_or_default(),
            model,
        }
    }

    /// Note the model a `turn_context` line names.
    pub fn context(&mut self, v: &Value) {
        if v.get("type").and_then(Value::as_str) == Some("turn_context") {
            if let Some(m) = v
                .pointer("/payload/model")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
            {
                self.model = Some(m.to_string());
            }
        }
    }

    /// The usage a new total adds to the one before. A total that went down (the thread was
    /// rewound) starts again from there and adds nothing.
    pub fn step(&mut self, total: Usage, ts: i64) -> Option<UsageMsg> {
        let prev = std::mem::replace(&mut self.prev, total);
        let t = total.since(&prev)?.tokens();
        let msg = UsageMsg {
            ts,
            model: self.model.clone().unwrap_or_default(),
            input: t.input,
            output: t.output,
            cache_write: t.cache_write,
            cache_read: t.cache_read,
        };
        (!msg.is_empty()).then_some(msg)
    }
}

/// The limits a `token_count` line reports, None when it reports none.
pub fn limits_of(v: &Value, ts: i64) -> Option<PlanLimits> {
    let p = v.get("payload")?;
    if p.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let r = p.get("rate_limits").filter(|r| r.is_object())?;
    let windows: Vec<LimitWindow> = ["primary", "secondary"]
        .iter()
        .filter_map(|name| {
            let w = r.get(*name).filter(|w| w.is_object())?;
            Some(LimitWindow {
                name: (*name).to_string(),
                used_percent: w.get("used_percent")?.as_f64()?,
                window_minutes: w.get("window_minutes").and_then(Value::as_i64),
                resets_at: w.get("resets_at").and_then(Value::as_i64).map(|s| s * 1000),
            })
        })
        .collect();
    (!windows.is_empty()).then(|| PlanLimits {
        seen_at: ts,
        plan: r.get("plan_type").and_then(Value::as_str).map(String::from),
        windows,
    })
}

/// The usage of every step of the whole rollout at `path`.
pub fn read_msgs(path: &Path) -> Result<Vec<UsageMsg>> {
    let mut steps = Steps::default();
    let mut msgs = vec![];
    for_each_line(path, 0, |line| {
        if !line.windows(11).any(|w| w == b"token_usage")
            && !line.windows(12).any(|w| w == b"turn_context")
        {
            return;
        }
        let Some((v, event)) = read_line(line) else {
            return;
        };
        steps.context(&v);
        if let Event::Total(u) = event {
            let ts = v
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(iso_to_ms)
                .unwrap_or(0);
            msgs.extend(steps.step(u, ts));
        }
    })?;
    Ok(msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn total(input: i64, cached: i64, output: i64) -> Value {
        json!({"timestamp":"2026-11-01T10:00:00Z","type":"event_msg","payload":{"type":"token_count",
            "info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":cached,"cache_write_input_tokens":0,"output_tokens":output}}}})
    }

    fn rollout(lines: &[Value]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn each_step_is_the_growth_of_the_total_with_the_model_of_its_turn() {
        let turn = |m: &str| json!({"timestamp":"2026-11-01T09:00:00Z","type":"turn_context","payload":{"model":m}});
        let (_d, path) = rollout(&[
            turn("gpt-5"),
            total(100, 20, 10),
            total(100, 20, 10), // a repeated total adds nothing
            turn("gpt-6-luna"),
            total(300, 120, 40),
            total(50, 0, 5), // the thread was rewound: starts again, adds nothing
            total(80, 0, 9),
        ]);
        let m = read_msgs(&path).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(
            (
                m[0].model.as_str(),
                m[0].input,
                m[0].cache_read,
                m[0].output
            ),
            ("gpt-5", 80, 20, 10)
        );
        assert_eq!(
            (
                m[1].model.as_str(),
                m[1].input,
                m[1].cache_read,
                m[1].output
            ),
            ("gpt-6-luna", 100, 100, 30)
        );
        assert_eq!((m[2].input, m[2].output), (30, 4));
    }

    #[test]
    fn a_resumed_read_goes_on_from_the_total_and_model_it_was_given() {
        let mut steps = Steps::resume(
            Some(Usage {
                input: 100,
                cached: 0,
                cache_write: 0,
                output: 10,
            }),
            Some("gpt-5".into()),
        );
        let m = steps
            .step(
                Usage {
                    input: 150,
                    cached: 0,
                    cache_write: 0,
                    output: 12,
                },
                7,
            )
            .unwrap();
        assert_eq!(
            (m.input, m.output, m.ts, m.model.as_str()),
            (50, 2, 7, "gpt-5")
        );
    }

    #[test]
    fn rate_limits_become_windows_and_a_null_one_is_none() {
        let v = json!({"payload":{"type":"token_count","rate_limits":{"plan_type":"plus",
            "primary":{"used_percent":23.0,"window_minutes":300,"resets_at":1_800_000_000},
            "secondary":{"used_percent":41.5,"window_minutes":10080,"resets_at":1_800_500_000}}}});
        let l = limits_of(&v, 5).unwrap();
        assert_eq!(l.seen_at, 5);
        assert_eq!(l.plan.as_deref(), Some("plus"));
        assert_eq!(l.windows.len(), 2);
        assert_eq!(l.windows[1].used_percent, 41.5);
        assert_eq!(l.windows[1].resets_at, Some(1_800_500_000_000));
        let none = json!({"payload":{"type":"token_count","rate_limits":null}});
        assert!(limits_of(&none, 5).is_none());
        let both_null = json!({"payload":{"type":"token_count","rate_limits":{"primary":null,"secondary":null}}});
        assert!(limits_of(&both_null, 5).is_none());
    }
}
