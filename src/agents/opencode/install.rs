//! OpenCode's side of `eye install | uninstall | doctor`: the plugin that reports live status
//! (`<config>/plugins/agent-eye.js`), the `/eye` command (`<config>/commands/eye.md`) and the
//! checks that OpenCode's files still look as eye expects. OpenCode's own config file
//! (`opencode.json`) and every other plugin and command are never touched.

use super::OpenCode;
use crate::agents::skill_file;
use crate::agents::{Agent, Check, InstallCtx};
use crate::report::{Report, Row};
use crate::util::show_path;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// The `/eye` command: a markdown file whose body is the instructions, `$ARGUMENTS` what follows it.
pub(crate) const COMMAND_MD: &str = include_str!("../../../skill/SKILL.opencode.md");
const PLUGIN_JS: &str = include_str!("plugin.js");
/// The line that says a plugin file is eye's.
pub(crate) const MARKER: &str = "// agent-eye-plugin: ";
/// Said once the files are written.
const RELOAD_NOTE: &str =
    "restart OpenCode (or run `opencode reload`) to load eye's plugin and the /eye command";

/// The plugin with the path of `agent-eye` in it, as a JavaScript string.
pub(crate) fn plugin_text(eye_bin: &Path) -> String {
    let quoted =
        serde_json::to_string(&eye_bin.to_string_lossy()).unwrap_or_else(|_| "\"\"".into());
    PLUGIN_JS.replace("\"/__EYE_BIN__\"", &quoted)
}

impl OpenCode {
    /// `<config>/plugins/agent-eye.js`.
    pub fn plugin_path(&self) -> PathBuf {
        self.config_dir.join("plugins").join("agent-eye.js")
    }

    /// `<config>/commands/eye.md`.
    pub fn command_path(&self) -> PathBuf {
        self.config_dir.join("commands").join("eye.md")
    }
}

/// Is the file at `path` one eye wrote? A file of somebody else's with the same name is theirs.
fn is_ours(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.contains(MARKER))
}

/// Checked before anything is written: the plugin's name must not be taken by another file.
pub fn preflight(c: &OpenCode, _ctx: &InstallCtx) -> Result<()> {
    let p = c.plugin_path();
    anyhow::ensure!(
        !p.exists() || is_ours(&p),
        "{} exists and is not eye's plugin; move it away and run install again",
        show_path(&p)
    );
    let cmd = c.command_path();
    anyhow::ensure!(
        !cmd.exists()
            || std::fs::read_to_string(&cmd)
                .is_ok_and(|t| t == COMMAND_MD || t.contains("`eye find --json")),
        "{} exists and is not eye's /eye command; move it away and run install again",
        show_path(&cmd)
    );
    Ok(())
}

pub fn install(c: &OpenCode, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    if !c.detect().installed {
        rep.push(Row::new(
            super::ID,
            format!(
                "not found (no {} and no `opencode` on PATH); plugin and command skipped, run install again once it is there",
                show_path(&c.dir)
            ),
        ));
        return Ok(rep);
    }
    let plugin = skill_file::install(&c.plugin_path(), &plugin_text(ctx.eye_bin), dry)?;
    let command = skill_file::install(&c.command_path(), COMMAND_MD, dry)?;
    rep.push(Row::new(
        super::ID,
        format!(
            "{}, {}",
            plugin.said(dry).replace("skill", "plugin"),
            command.said(dry).replace("skill", "/eye command")
        ),
    ));
    if plugin != skill_file::State::Unchanged || command != skill_file::State::Unchanged {
        rep.next.push(RELOAD_NOTE.into());
    }
    Ok(rep)
}

pub fn uninstall(c: &OpenCode, ctx: &InstallCtx) -> Result<Report> {
    let mut rep = Report::default();
    let dry = ctx.dry_run;
    let (pp, cp) = (c.plugin_path(), c.command_path());
    if !pp.exists() && !cp.exists() {
        return Ok(rep);
    }
    let plugin = is_ours(&pp) && skill_file::remove(&pp, dry)?;
    let command = std::fs::read_to_string(&cp).is_ok_and(|t| t == COMMAND_MD)
        && skill_file::remove(&cp, dry)?;
    // the folders go when eye's file was the only one in them
    if !dry {
        for d in [pp.parent(), cp.parent()].into_iter().flatten() {
            let _ = std::fs::remove_dir(d);
        }
    }
    let word = |removed: bool, what: &str| match (removed, dry) {
        (true, false) => format!("{what} removed"),
        (true, true) => format!("would remove {what}"),
        (false, _) => format!("no {what} of eye's found"),
    };
    rep.push(Row::new(
        super::ID,
        format!(
            "{}, {}",
            word(plugin, "plugin"),
            word(command, "/eye command")
        ),
    ));
    Ok(rep)
}

pub fn doctor(c: &OpenCode, ctx: &InstallCtx) -> Vec<Check> {
    let mut out = vec![];
    let det = c.detect();
    if !det.installed {
        out.push(Check::warn(format!(
            "OpenCode not found: {} (turn it off with `enabled = [\"claude\"]` under [agents] in config.toml)",
            det.detail
        )));
        return out;
    }
    let pp = c.plugin_path();
    out.push(match std::fs::read_to_string(&pp) {
        Ok(t) if !t.contains(MARKER) => {
            Check::warn(format!("{} is not eye's plugin", show_path(&pp)))
        }
        Ok(t) if t == plugin_text(ctx.eye_bin) => Check::ok(format!(
            "plugin in place ({}): live status and permission prompts reach eye",
            show_path(&pp)
        )),
        Ok(_) => Check::warn(format!(
            "plugin is out of date or names another agent-eye (run `{} install`)",
            crate::pick::bin_name()
        )),
        Err(_) => Check::warn(format!(
            "plugin missing: live status will not see permission prompts (run `{} install`)",
            crate::pick::bin_name()
        )),
    });
    let cp = c.command_path();
    out.push(match std::fs::read_to_string(&cp) {
        Ok(t) if t == COMMAND_MD => Check::ok("/eye command in place"),
        Ok(_) => Check::warn(format!(
            "/eye command is out of date (run `{} install`)",
            crate::pick::bin_name()
        )),
        Err(_) => Check::warn(format!(
            "/eye command missing (run `{} install`)",
            crate::pick::bin_name()
        )),
    });
    match c.output(&["--version"], None) {
        Ok(v) => out.push(Check::ok(format!("opencode {}", v.trim()))),
        Err(_) => out.push(Check::warn("could not run `opencode --version`")),
    }
    out.push(match super::headless::flags_check(c) {
        Ok(()) => Check::ok("`opencode run` has the flags summaries need"),
        Err(e) => Check::warn(format!("summaries: {e:#}")),
    });
    let n = super::store::scan(&c.dir).len();
    out.push(if n > 0 {
        Check::ok(format!("{n} sessions found under {}", show_path(&c.dir)))
    } else {
        Check::warn(format!("no sessions under {}", show_path(&c.dir)))
    });
    out
}
