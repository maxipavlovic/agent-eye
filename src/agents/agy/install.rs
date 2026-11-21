//! agy's side of `eye install | uninstall | doctor`.
//!
//! agy reads hooks from a `hooks.json` in a workspace's `.agents/` folder or in a plugin, and
//! plugins and skills from the global folder `~/.gemini/config/` too (checked on agy 1.5.14: a
//! plugin there with a `hooks.json` runs its hooks in every conversation, and `agy plugin list`
//! shows it). eye installs its own plugin `agent-eye` there, so nobody else's hooks are touched:
//!
//! ```text
//! ~/.gemini/config/plugins/agent-eye/plugin.json
//! ~/.gemini/config/plugins/agent-eye/hooks.json   SessionStart, Stop
//! ~/.gemini/config/skills/eye/SKILL.md            the /eye skill
//! ```
//!
//! agy waits for a hook, and a `PreToolUse` hook that prints no decision can block the tool, so eye
//! listens to no tool event. The command ends in `echo '{}'` whatever eye did: an empty decision
//! lets agy go on, and a missing or failing `agent-eye` can never stall it.

use super::{hooks, Agy};
use crate::agents::skill_file::{self, State};
use crate::agents::{Agent, Check, InstallCtx, ParseOpts};
use crate::report::{Report, Row};
use crate::util::{shell_quote, show_path};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

pub(crate) const SKILL_MD: &str = include_str!("../../../skill/SKILL.agy.md");
/// The plugin's name and its folder.
pub const PLUGIN: &str = "agent-eye";

/// The command of one hook: eye's, then an empty JSON decision whatever happened.
pub fn command(bin: &Path, event: &str) -> String {
    let b = bin.to_string_lossy();
    let b = if b.contains(|c: char| c.is_whitespace() || c == '\'') {
        shell_quote(&b)
    } else {
        b.into_owned()
    };
    format!(
        "{{ {b} hook {event} --agent {}; }} >/dev/null 2>&1; echo '{{}}'",
        super::ID
    )
}

/// The text of the plugin's `hooks.json`.
pub fn hooks_json(bin: &Path) -> String {
    let mut m = serde_json::Map::new();
    for e in hooks::EVENTS {
        m.insert(
            e.into(),
            json!([{"type": "command", "command": command(bin, e), "timeout": 5}]),
        );
    }
    let mut text = serde_json::to_string_pretty(&json!({ PLUGIN: m })).unwrap_or_default();
    text.push('\n');
    text
}

fn manifest() -> String {
    let v = json!({"name": PLUGIN, "description": "Passes agy's session events to agent-eye (eye), which shows what needs you."});
    let mut t = serde_json::to_string_pretty(&v).unwrap_or_default();
    t.push('\n');
    t
}

/// Is the plugin folder eye's (or not there)? Another plugin of the same name is left alone.
fn ours(a: &Agy) -> Result<()> {
    let p = a.plugin_dir().join("plugin.json");
    let Ok(text) = std::fs::read_to_string(&p) else {
        return Ok(());
    };
    let name = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("name").and_then(Value::as_str).map(String::from));
    if name.as_deref() != Some(PLUGIN) {
        bail!(
            "{} is a plugin that is not eye's; eye does not touch it",
            show_path(&p)
        );
    }
    Ok(())
}

pub fn preflight(a: &Agy, _ctx: &InstallCtx) -> Result<()> {
    ours(a)
}

fn merge(states: [State; 3]) -> State {
    if states.iter().all(|s| *s == State::Unchanged) {
        State::Unchanged
    } else if states.contains(&State::Updated) {
        State::Updated
    } else {
        State::Added
    }
}

pub fn install(a: &Agy, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    if !a.detect().installed {
        rep.push(Row::new(
            super::ID,
            format!(
                "not found (no {} and no `agy` on PATH); hooks and skill skipped, run install again once it is there",
                show_path(&a.dir)
            ),
        ));
        return Ok(rep);
    }
    ours(a)?;
    let manifest = skill_file::install(&a.plugin_dir().join("plugin.json"), &manifest(), dry)?;
    let hooks = skill_file::install(
        &a.plugin_dir().join("hooks.json"),
        &hooks_json(ctx.eye_bin),
        dry,
    )?;
    let skill = skill_file::install(&a.skill_path(), SKILL_MD, dry)?;
    let hooks_said = match (merge([manifest, hooks, State::Unchanged]), dry) {
        (State::Unchanged, _) => format!("hooks ok in plugin {PLUGIN}"),
        (State::Added, false) => format!("hooks added (plugin {PLUGIN})"),
        (State::Added, true) => format!("would add hooks (plugin {PLUGIN})"),
        (_, false) => format!("hooks updated (plugin {PLUGIN})"),
        (_, true) => format!("would update hooks (plugin {PLUGIN})"),
    };
    rep.push(Row::new(
        super::ID,
        format!("{hooks_said}, {}", skill.said(dry)),
    ));
    Ok(rep)
}

pub fn uninstall(a: &Agy, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    let (plugin, skill_path) = (a.plugin_dir(), a.skill_path());
    if !plugin.exists() && !skill_path.exists() {
        return Ok(rep);
    }
    let mut hooks = false;
    if plugin.exists() {
        ours(a)?;
        for f in ["hooks.json", "plugin.json"] {
            hooks |= skill_file::remove(&plugin.join(f), dry)?;
        }
    }
    let skill = skill_file::remove(&skill_path, dry)?;
    let hooks_said = match (hooks, dry) {
        (true, false) => "hooks removed",
        (true, true) => "would remove hooks",
        (false, _) => "no hooks found",
    };
    rep.push(Row::new(
        super::ID,
        format!("{hooks_said}, {}", skill_file::removed_said(skill, dry)),
    ));
    Ok(rep)
}

/// Does the plugin's `hooks.json` have an eye hook for every event?
fn missing_events(a: &Agy) -> Result<Vec<&'static str>> {
    let text = std::fs::read_to_string(a.plugin_dir().join("hooks.json"))?;
    let v: Value = serde_json::from_str(&text)?;
    Ok(hooks::EVENTS
        .into_iter()
        .filter(|e| {
            !v.get(PLUGIN)
                .and_then(|p| p.get(*e))
                .and_then(Value::as_array)
                .is_some_and(|hs| {
                    hs.iter().any(|h| {
                        h.get("command")
                            .and_then(Value::as_str)
                            .is_some_and(crate::agents::hooks_file::is_eye_hook_command)
                    })
                })
        })
        .collect())
}

pub fn doctor(a: &Agy, _ctx: &InstallCtx) -> Vec<Check> {
    let mut out = vec![];
    let det = a.detect();
    if !det.installed {
        out.push(Check::warn(format!(
            "Antigravity CLI not found: {} (turn it off with `enabled = [\"claude\"]` under [agents] in config.toml)",
            det.detail
        )));
        return out;
    }
    match missing_events(a) {
        Ok(m) if m.is_empty() => out.push(Check::ok(format!(
            "hooks present for both events in plugin {PLUGIN} ({})",
            show_path(&a.plugin_dir())
        ))),
        Ok(m) => out.push(Check::warn(format!(
            "hooks missing: {} (run `{} install`)",
            m.join(", "),
            crate::pick::bin_name()
        ))),
        Err(_) => out.push(Check::warn(format!(
            "no hooks in plugin {PLUGIN} (run `{} install`)",
            crate::pick::bin_name()
        ))),
    }
    match Command::new(a.executable_path()).arg("--version").output() {
        Ok(r) if r.status.success() => out.push(Check::ok(
            String::from_utf8_lossy(&r.stdout).trim().to_string(),
        )),
        _ => out.push(Check::warn("could not run `agy --version`")),
    }
    let sums = super::files::summaries(&a.dir);
    if sums.is_empty() {
        out.push(Check::warn(format!(
            "no rows in {}: conversation_summaries.db missing or unreadable",
            show_path(&a.dir.join("conversation_summaries.db"))
        )));
    }
    let (found, _) = super::files::scan(&a.dir);
    match found.iter().max_by_key(|f| f.stamp.mtime) {
        Some(f) => {
            let tokens = HashSet::new();
            let opts = ParseOpts {
                job_tokens: &tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            };
            match a.parse_transcript(f, &opts) {
                Ok(d) if d.lines > 0 && (d.prompt_count > 0 || d.cwd.is_some()) => {
                    out.push(Check::ok(format!(
                        "newest conversation parses ({} steps, {} prompts, {} replies)",
                        d.lines, d.prompt_count, d.reply_count
                    )))
                }
                Ok(_) => out.push(Check::warn(format!(
                    "newest conversation {} has no prompt or folder; format changed?",
                    show_path(&f.path)
                ))),
                Err(e) => out.push(Check::warn(format!(
                    "newest conversation unreadable: {e:#}"
                ))),
            }
        }
        None => out.push(Check::warn(format!(
            "no conversations under {}",
            show_path(&a.dir.join(super::files::BRAIN))
        ))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hook_ends_in_an_empty_decision_and_names_the_agent() {
        let text = hooks_json(Path::new("/Users/you/.local/bin/agent-eye"));
        let v: Value = serde_json::from_str(&text).unwrap();
        let stop = &v[PLUGIN]["Stop"][0];
        assert_eq!(stop["type"], "command");
        assert_eq!(stop["timeout"], 5);
        let c = stop["command"].as_str().unwrap();
        assert_eq!(
            c,
            "{ /Users/you/.local/bin/agent-eye hook Stop --agent agy; } >/dev/null 2>&1; echo '{}'"
        );
        assert!(crate::agents::hooks_file::is_eye_hook_command(c));
        assert_eq!(v[PLUGIN].as_object().unwrap().len(), 2);
        assert!(v[PLUGIN].get("PreToolUse").is_none());
        // a path with a space is quoted
        assert!(command(Path::new("/a b/agent-eye"), "Stop").starts_with("{ '/a b/agent-eye' hook"));
    }

    #[test]
    fn the_skill_has_the_front_matter_agy_needs_and_names_no_other_agent() {
        let (front, _) = SKILL_MD
            .strip_prefix("---\n")
            .and_then(|r| r.split_once("\n---\n"))
            .expect("front matter");
        assert!(front.contains("name: eye\n") && front.contains("\ndescription: "));
        assert!(!SKILL_MD.contains("Claude") && !SKILL_MD.contains("Codex"));
        assert!(SKILL_MD.contains("Antigravity"));
    }
}
