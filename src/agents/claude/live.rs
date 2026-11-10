//! Readers for `sessions/*.json` (open sessions) and `jobs/*/state.json` (background sessions).

use super::super::{Job, LiveSession};
use crate::util::{iso_to_ms, pid_alive};
use serde_json::Value;
use std::path::Path;

fn ms_value(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => iso_to_ms(s).or_else(|| s.parse().ok()),
        _ => None,
    }
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Only files named `<digits>.json` are opened; `*.key` and `*-session.tmp` never are.
pub fn read_sessions(dir: &Path) -> Vec<LiveSession> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("");
        if p.extension().and_then(|x| x.to_str()) != Some("json")
            || stem.is_empty()
            || !stem.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let Ok(txt) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&txt) else {
            continue;
        };
        let Some(session_id) = str_of(&v, "sessionId") else {
            continue;
        };
        let pid = v.get("pid").and_then(Value::as_i64).unwrap_or(0);
        out.push(LiveSession {
            pid,
            session_id,
            cwd: str_of(&v, "cwd"),
            status: str_of(&v, "status"),
            updated_at: ms_value(v.get("updatedAt")),
            entrypoint: str_of(&v, "entrypoint"),
            alive: pid_alive(pid),
        });
    }
    out
}

/// Background sessions: one `jobs/<short id>/state.json` each.
pub fn read_jobs(dir: &Path) -> Vec<Job> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut out = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let Some(short) = p.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        let Ok(txt) = std::fs::read_to_string(p.join("state.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&txt) else {
            continue;
        };
        out.push(Job {
            short,
            state: str_of(&v, "state").unwrap_or_default(),
            detail: str_of(&v, "detail"),
            name: str_of(&v, "name"),
            session_id: str_of(&v, "sessionId"),
            resume_session_id: str_of(&v, "resumeSessionId"),
            cwd: str_of(&v, "cwd"),
            created_at: ms_value(v.get("createdAt")),
            updated_at: ms_value(v.get("updatedAt")),
        });
    }
    out.sort_by(|a, b| a.short.cmp(&b.short));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_only() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("1.json"),
            r#"{"pid":1,"sessionId":"s1","cwd":"/x","status":"busy","updatedAt":5}"#,
        )
        .unwrap();
        std::fs::write(d.path().join("1.key"), "SECRET").unwrap();
        std::fs::write(d.path().join("x-session.tmp"), "{}").unwrap();
        std::fs::write(d.path().join("bad.json"), "{{").unwrap();
        let s = read_sessions(d.path());
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].session_id, "s1");
        assert_eq!(s[0].updated_at, Some(5));
    }

    #[test]
    fn reads_jobs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("abcd1234")).unwrap();
        std::fs::write(
            d.path().join("abcd1234/state.json"),
            r#"{"state":"done","detail":"ok","sessionId":"s2","createdAt":"2026-11-01T10:00:00Z","name":"n"}"#,
        )
        .unwrap();
        std::fs::write(d.path().join("pins.json"), "[]").unwrap();
        let j = read_jobs(d.path());
        assert_eq!(j.len(), 1);
        assert_eq!(j[0].short, "abcd1234");
        assert_eq!(j[0].created_at, Some(1_793_527_200_000));
    }
}
