//! Claude Code's side of `eye install | uninstall | doctor`: the hooks in `settings.json`, the
//! `/eye` skill, and the checks that Claude Code's files still look as eye expects. Other tools'
//! hook entries in `settings.json` are never touched.

use super::{Claude, ID};
use crate::agents::hooks_file::{self, Change};
use crate::agents::skill_file;
use crate::agents::{Agent, Check, InstallCtx, ParseOpts, Transcript};
use crate::report::{Report, Row};
use crate::util::show_path;
use anyhow::Result;
use serde_json::json;
use std::collections::HashSet;
use std::process::Command;

/// The hook events eye listens to.
pub const EVENTS: [&str; 8] = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SubagentStart",
    "SubagentStop",
    "SessionEnd",
];
const SKILL_MD: &str = include_str!("../../../skill/SKILL.md");

/// What Claude Code's `settings.json` looks like to eye: one group per event, the hook command
/// without an agent argument (the form installed before there were several agents).
const SPEC: hooks_file::Spec = hooks_file::Spec {
    file: "settings.json",
    events: &EVENTS,
    agent: None,
    group: |command, _| {
        hooks_file::group(
            &[("matcher", json!(""))],
            json!({"type": "command", "command": command, "async": true, "timeout": 5}),
        )
    },
};

/// Checked before anything is written: `settings.json` must be JSON eye can add its hooks to.
pub fn preflight(c: &Claude, ctx: &InstallCtx) -> Result<()> {
    let (mut settings, _) = hooks_file::read(&c.settings_path())?;
    hooks_file::merge(&mut settings, ctx.eye_bin, &SPEC).map(drop)
}

/// Add eye's hooks to `settings.json` and write the `/eye` skill.
pub fn install(c: &Claude, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    if !c.detect().installed {
        rep.push(Row::new(
            ID,
            format!(
                "not found (no {} and no `claude` on PATH); hooks and skill skipped, run install again once it is there",
                show_path(&c.dir)
            ),
        ));
        return Ok(rep);
    }
    let settings_path = c.settings_path();
    let (before, old_text) = hooks_file::read(&settings_path)?;
    let mut settings = before.clone();
    let merged = hooks_file::merge(&mut settings, ctx.eye_bin, &SPEC)?;
    let change = if !merged.changed() {
        Change::default()
    } else {
        hooks_file::change(&settings_path, &SPEC, &before, &old_text, &settings, dry)?
    };
    let skill = skill_file::install(&c.skill_path(), SKILL_MD, dry)?;
    rep.push(
        Row::new(
            ID,
            format!(
                "{}, {}",
                hooks_file::added_text(&SPEC, &settings_path, &merged, &change, dry),
                skill.said(dry)
            ),
        )
        .with_detail(change.diff),
    );
    Ok(rep)
}

/// Remove eye's hooks and the skill. A machine with neither gets no row.
pub fn uninstall(c: &Claude, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    let settings_path = c.settings_path();
    let skill_path = c.skill_path();
    if !settings_path.exists() && !skill_path.exists() {
        return Ok(rep);
    }
    let (before, old_text) = hooks_file::read(&settings_path)?;
    let mut settings = before.clone();
    let removed = hooks_file::remove(&mut settings);
    let change = if removed == 0 {
        Change::default()
    } else {
        hooks_file::change(&settings_path, &SPEC, &before, &old_text, &settings, dry)?
    };
    let skill = skill_file::remove(&skill_path, dry)?;
    rep.push(
        Row::new(
            ID,
            format!(
                "{}, {}",
                hooks_file::removed_text(&settings_path, removed, &change, dry),
                skill_file::removed_said(skill, dry)
            ),
        )
        .with_detail(change.diff),
    );
    Ok(rep)
}

fn newest_transcript(files: Vec<Transcript>) -> Option<Transcript> {
    files.into_iter().max_by_key(|f| f.stamp.mtime)
}

/// Check that Claude Code's files still look as eye expects.
pub fn doctor(c: &Claude, _ctx: &InstallCtx) -> Vec<Check> {
    let mut out = vec![];
    let det = c.detect();
    if !det.installed {
        out.push(Check::warn(format!(
            "Claude Code not found: {} (turn it off with `enabled = []` under [agents] in config.toml)",
            det.detail
        )));
        return out;
    }
    match hooks_file::read(&c.settings_path()) {
        Ok((v, _)) => {
            let missing = hooks_file::missing(&v, &SPEC);
            if missing.is_empty() {
                out.push(Check::ok(format!(
                    "hooks present for all {} events",
                    EVENTS.len()
                )));
            } else {
                out.push(Check::warn(format!(
                    "hooks missing: {} (run `{} install`)",
                    missing.join(", "),
                    crate::pick::bin_name()
                )));
            }
        }
        Err(e) => out.push(Check::warn(format!("settings.json: {e:#}"))),
    }
    match Command::new(c.executable_path()).arg("--version").output() {
        Ok(r) if r.status.success() => out.push(Check::ok(format!(
            "Claude Code {}",
            String::from_utf8_lossy(&r.stdout).trim()
        ))),
        _ => out.push(Check::warn("could not run `claude --version`")),
    }
    match newest_transcript(super::transcript_files(&c.dir)) {
        Some(f) => {
            let tokens = HashSet::new();
            let opts = ParseOpts {
                job_tokens: &tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            };
            match super::transcript::parse_file(&f.path, 0, &opts) {
                Ok(d) if d.session_id.is_some() && d.lines > 0 => out.push(Check::ok(format!(
                    "newest transcript parses ({} lines, {} prompts, {} replies)",
                    d.lines, d.prompt_count, d.reply_count
                ))),
                Ok(_) => out.push(Check::warn(format!(
                    "newest transcript {} has no session id; format changed?",
                    f.path.display()
                ))),
                Err(e) => out.push(Check::warn(format!("newest transcript unreadable: {e:#}"))),
            }
        }
        None => out.push(Check::warn(format!(
            "no transcripts under {}",
            c.dir.join("projects").display()
        ))),
    }
    let sessions_dir = c.dir.join("sessions");
    let n_files = std::fs::read_dir(&sessions_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.strip_suffix(".json")
                        .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit()) && !s.is_empty())
                })
                .count()
        })
        .unwrap_or(0);
    let parsed = super::live::read_sessions(&sessions_dir).len();
    let line = format!("sessions/*.json: {parsed} of {n_files} files parse");
    out.push(if parsed == n_files {
        Check::ok(line)
    } else {
        Check::warn(line)
    });
    let jobs_dir = c.dir.join("jobs");
    let n_jobs = std::fs::read_dir(&jobs_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("state.json").exists())
                .count()
        })
        .unwrap_or(0);
    let parsed = super::live::read_jobs(&jobs_dir).len();
    let line = format!("jobs/*/state.json: {parsed} of {n_jobs} parse");
    out.push(if parsed == n_jobs {
        Check::ok(line)
    } else {
        Check::warn(line)
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_hook_entry_keeps_the_form_on_installed_machines() {
        let mut v = json!({});
        hooks_file::merge(&mut v, Path::new("/Users/you/.local/bin/agent-eye"), &SPEC).unwrap();
        assert_eq!(
            v["hooks"]["Stop"][0],
            json!({
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": "/Users/you/.local/bin/agent-eye hook Stop",
                    "async": true,
                    "timeout": 5
                }]
            })
        );
        assert_eq!(hooks_file::missing(&v, &SPEC).len(), 0);
        assert_eq!(v["hooks"].as_object().unwrap().len(), EVENTS.len());
    }
}
