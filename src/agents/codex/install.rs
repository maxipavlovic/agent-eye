//! Codex's side of `eye install | uninstall | doctor`: the hooks in `hooks.json`, the `/eye`
//! skill, and the checks that Codex's files still look as eye expects. Other tools' hooks in
//! `hooks.json` are never touched, and neither is the `notify` setting in `config.toml`.

use super::{rollout, Codex};
use crate::agents::hooks_file::{self, Change};
use crate::agents::skill_file;
use crate::agents::{Agent, Check, InstallCtx, ParseOpts};
use crate::report::{Report, Row};
use crate::util::show_path;
use anyhow::Result;
use serde_json::json;
use std::collections::HashSet;
use std::process::Command;

/// The hook events eye listens to. A sub-agent's own events say nothing eye shows.
pub const EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "PostToolUse",
    "Stop",
    "SessionEnd",
];
const SKILL_MD: &str = include_str!("../../../skill/SKILL.codex.md");

/// What Codex's `hooks.json` looks like to eye: the form Claude Code uses, a command per event
/// that names the agent. `statusMessage` is the name Codex shows in `/hooks` (and, briefly, while
/// the hook runs). A `SessionEnd` hook runs before Codex exits, so it is not asked to be
/// asynchronous.
const SPEC: hooks_file::Spec = hooks_file::Spec {
    file: "hooks.json",
    events: &EVENTS,
    agent: Some(super::ID),
    group: |command, event| {
        let mut handler =
            json!({"type": "command", "command": command, "statusMessage": "agent-eye"});
        if event != "SessionEnd" {
            handler["async"] = json!(true);
        }
        handler["timeout"] = json!(5);
        hooks_file::group(&[], handler)
    },
};

/// What Codex does with a hook it has not seen before, said when hooks were added.
const TRUST_NOTE: &str =
    "start Codex and run /hooks once to approve eye's hooks (Codex asks again when a hook changes)";

/// Checked before anything is written: `hooks.json` must be JSON eye can add its hooks to.
pub fn preflight(c: &Codex, ctx: &InstallCtx) -> Result<()> {
    let (mut hooks, _) = hooks_file::read(&c.hooks_path())?;
    hooks_file::merge(&mut hooks, ctx.eye_bin, &SPEC).map(drop)
}

/// Add eye's hooks to `hooks.json` and write the `/eye` skill.
pub fn install(c: &Codex, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    if !c.detect().installed {
        rep.push(Row::new(
            super::ID,
            format!(
                "not found (no {} and no `codex` on PATH); hooks and skill skipped, run install again once it is there",
                show_path(&c.dir)
            ),
        ));
        return Ok(rep);
    }
    let path = c.hooks_path();
    let (before, old_text) = hooks_file::read(&path)?;
    let mut hooks = before.clone();
    let merged = hooks_file::merge(&mut hooks, ctx.eye_bin, &SPEC)?;
    let change = if !merged.changed() {
        Change::default()
    } else {
        hooks_file::change(&path, &SPEC, &before, &old_text, &hooks, dry)?
    };
    let skill = skill_file::install(&c.skill_path(), SKILL_MD, dry)?;
    rep.push(
        Row::new(
            super::ID,
            format!(
                "{}, {}",
                hooks_file::added_text(&SPEC, &path, &merged, &change, dry),
                skill.said(dry)
            ),
        )
        .with_detail(change.diff),
    );
    if merged.changed() {
        rep.next.push(TRUST_NOTE.into());
    }
    Ok(rep)
}

/// Remove eye's hooks and the skill. A machine without either gets no row.
pub fn uninstall(c: &Codex, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    let path = c.hooks_path();
    let skill_path = c.skill_path();
    if !path.exists() && !skill_path.exists() {
        return Ok(rep);
    }
    let (before, old_text) = hooks_file::read(&path)?;
    let mut hooks = before.clone();
    let removed = hooks_file::remove(&mut hooks);
    let change = if removed == 0 {
        Change::default()
    } else {
        hooks_file::change(&path, &SPEC, &before, &old_text, &hooks, dry)?
    };
    let skill = skill_file::remove(&skill_path, dry)?;
    rep.push(
        Row::new(
            super::ID,
            format!(
                "{}, {}",
                hooks_file::removed_text(&path, removed, &change, dry),
                skill_file::removed_said(skill, dry)
            ),
        )
        .with_detail(change.diff),
    );
    Ok(rep)
}

/// Check that Codex's files still look as eye expects.
pub fn doctor(c: &Codex, _ctx: &InstallCtx) -> Vec<Check> {
    let mut out = vec![];
    let det = c.detect();
    if !det.installed {
        out.push(Check::warn(format!(
            "Codex CLI not found: {} (turn it off with `enabled = [\"claude\"]` under [agents] in config.toml)",
            det.detail
        )));
        return out;
    }
    match hooks_file::read(&c.hooks_path()) {
        Ok((v, _)) => {
            let missing = hooks_file::missing(&v, &SPEC);
            if missing.is_empty() {
                out.push(Check::ok(format!(
                    "hooks present for all {} events in hooks.json (whether you trusted them with /hooks in Codex, eye cannot see)",
                    EVENTS.len()
                )));
            } else {
                out.push(Check::warn(format!(
                    "hooks missing: {} (run `{} install`, then trust them with /hooks in Codex)",
                    missing.join(", "),
                    crate::pick::bin_name()
                )));
            }
        }
        Err(e) => out.push(Check::warn(format!("hooks.json: {e:#}"))),
    }
    match Command::new(c.executable_path()).arg("--version").output() {
        Ok(r) if r.status.success() => out.push(Check::ok(
            String::from_utf8_lossy(&r.stdout).trim().to_string(),
        )),
        _ => out.push(Check::warn("could not run `codex --version`")),
    }
    out.push(match super::headless::flags_check(&c.executable_path()) {
        Ok(()) => Check::ok("`codex exec` has the flags summaries need"),
        Err(e) => Check::warn(format!("summaries: {e:#}")),
    });
    let (files, _) = super::files::scan(&c.dir);
    match files.iter().max_by_key(|f| f.stamp.mtime) {
        Some(f) => {
            let tokens = HashSet::new();
            let opts = ParseOpts {
                job_tokens: &tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            };
            match rollout::parse_file(&f.path, 0, &f.session_id, &opts, false) {
                Ok(p)
                    if p.delta.lines > 0 && (p.delta.prompt_count > 0 || p.delta.cwd.is_some()) =>
                {
                    out.push(Check::ok(format!(
                        "newest rollout parses ({} lines, {} prompts, {} replies)",
                        p.delta.lines, p.delta.prompt_count, p.delta.reply_count
                    )))
                }
                Ok(_) => out.push(Check::warn(format!(
                    "newest rollout {} has no prompt or folder; format changed?",
                    f.path.display()
                ))),
                Err(e) => out.push(Check::warn(format!("newest rollout unreadable: {e:#}"))),
            }
        }
        None => out.push(Check::warn(format!(
            "no rollouts under {}",
            c.dir.join(super::files::SESSIONS_DIR).display()
        ))),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_hook_entry_names_the_agent_and_a_session_end_is_not_async() {
        let mut v = json!({});
        hooks_file::merge(&mut v, Path::new("/Users/you/.local/bin/agent-eye"), &SPEC).unwrap();
        assert_eq!(v["hooks"].as_object().unwrap().len(), 6);
        assert_eq!(
            v["hooks"]["Stop"][0],
            json!({"hooks": [{
                "type": "command",
                "command": "/Users/you/.local/bin/agent-eye hook Stop --agent codex",
                "statusMessage": "agent-eye",
                "async": true,
                "timeout": 5
            }]})
        );
        let end = &v["hooks"]["SessionEnd"][0]["hooks"][0];
        assert!(end.get("async").is_none() && end["timeout"] == 5, "{end}");
        assert!(v["hooks"].get("Notification").is_none());
    }

    #[test]
    fn the_skill_has_the_front_matter_codex_needs() {
        let (front, _) = SKILL_MD
            .strip_prefix("---\n")
            .and_then(|r| r.split_once("\n---\n"))
            .expect("front matter");
        assert!(front.contains("name: eye\n") && front.contains("\ndescription: "));
        assert!(!SKILL_MD.contains("Claude"));
    }
}
