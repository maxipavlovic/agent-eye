//! `eye install | uninstall | doctor`: the binary, the `eye` alias and the background service (a
//! launch agent on macOS, a systemd user unit on Linux), with each agent's own hooks and skill
//! installed through its adapter. Every path can be overridden and `--dry-run` writes nothing.

mod choice;
mod package;
mod service;

pub use choice::{choose_agents, save_choice, Choice};
pub use service::{pause as pause_daemon, restart as restart_daemon, resume as resume_daemon};

use crate::agents::{self, Agents, Check, InstallCtx};
use crate::config::Config;
use crate::daemon;
use crate::files::write_file;
use crate::paths::Paths;
use crate::platform::service::{Manager, OldUnit, Runner, Timing, Unit};
use crate::platform::{account_home, search_path};
use crate::report::{Report, Row};
use crate::util::{ago, find_in, now_ms, show_path};
use anyhow::{Context, Result};
use package::Package;
use service::FileState;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The installed binary's name.
pub const BIN_NAME: &str = "agent-eye";
/// The short alias linked next to the binary.
pub const ALIAS_NAME: &str = "eye";

/// Where install, uninstall and doctor look, and what they may do.
#[derive(Debug, Clone)]
pub struct Opts {
    pub eye_home: PathBuf,
    pub bin_dir: PathBuf,
    /// The package manager whose folder `bin_dir` is, when eye runs from there and copies nothing.
    pub package: Option<Package>,
    /// Where earlier installs copied the binary (`~/.local/bin`), when `bin_dir` is a package
    /// manager's folder instead: a copy left there becomes a link to the package's binary.
    pub old_bin_dir: Option<PathBuf>,
    /// Which service manager keeps the daemon running.
    pub manager: Manager,
    /// Where the service file goes: the launch agents folder or the systemd user folder.
    pub service_dir: PathBuf,
    /// The binary to copy into `bin_dir`.
    pub exe: PathBuf,
    pub dry_run: bool,
    /// Do not run the service manager (it is only ever run for the real service folder anyway).
    pub no_service: bool,
    pub purge: bool,
    /// `install --board`: also turn the web board on.
    pub board: bool,
    /// Where the service manager's programs are looked up, when somebody gives a folder of their
    /// own (a test with stand-ins). Without it the real ones are used, and only for the real
    /// service folder.
    pub programs: Option<OsString>,
    /// How long to wait for the service manager.
    pub timing: Timing,
}

impl Opts {
    /// Options from the paths and the home folder, with the defaults for the rest.
    pub fn resolve(
        paths: &Paths,
        bin_dir: Option<PathBuf>,
        service_dir: Option<PathBuf>,
    ) -> Result<Opts> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let manager = Manager::current();
        let exe = std::env::current_exe().context("finding the eye binary")?;
        // Homebrew's or cargo's binary runs from their folder, so their upgrades reach the service
        // and the hooks; any other binary is copied to ~/.local/bin
        let local = home.join(".local/bin");
        let (bin_dir, package) = match bin_dir {
            Some(dir) => (dir, None),
            None => match package::owner(&exe, package::cargo_home().as_deref()) {
                Some((p, dir)) => (dir, Some(p)),
                None => (local.clone(), None),
            },
        };
        let old_bin_dir = package.and(Some(local)).filter(|d| *d != bin_dir);
        Ok(Opts {
            eye_home: paths.home.clone(),
            bin_dir,
            package,
            old_bin_dir,
            manager,
            service_dir: service_dir.unwrap_or_else(|| manager.default_dir(&home)),
            exe,
            dry_run: false,
            no_service: false,
            purge: false,
            board: false,
            programs: None,
            timing: Timing::REAL,
        })
    }
    fn eye_bin(&self) -> PathBuf {
        self.bin_dir.join(BIN_NAME)
    }
    fn alias_bin(&self) -> PathBuf {
        self.bin_dir.join(ALIAS_NAME)
    }
    /// The service file.
    fn unit(&self) -> PathBuf {
        self.service_dir.join(self.manager.file_name())
    }
    /// The service files of older installs that are there.
    fn old_units(&self) -> Vec<(PathBuf, OldUnit)> {
        self.manager
            .old_units()
            .into_iter()
            .map(|old| (self.service_dir.join(&old.file), old))
            .filter(|(file, _)| file.exists())
            .collect()
    }
    fn real_service_dir(&self) -> bool {
        account_home().is_some_and(|h| self.manager.default_dir(&h) == self.service_dir)
    }
    /// Does the installed service file run the daemon of this eye home? Another home's daemon is
    /// not ours to restart.
    fn unit_serves_home(&self) -> bool {
        let Ok(text) = std::fs::read_to_string(self.unit()) else {
            return false;
        };
        self.manager
            .serves_home(&text, &self.eye_home, account_home().as_deref())
    }
    /// Where the service manager's programs are looked up, when it may be run: never in a dry
    /// run, with `--no-service`, or for a service folder that is not the real one.
    fn service_path(&self) -> Option<OsString> {
        if self.dry_run || self.no_service {
            return None;
        }
        self.programs
            .clone()
            .or_else(|| self.real_service_dir().then(search_path))
    }
    /// Why the service manager is not run, for a message.
    fn not_run_because(&self) -> &'static str {
        if self.dry_run {
            "dry run"
        } else if self.no_service {
            "--no-service"
        } else {
            "the service folder is not the default one"
        }
    }
}

/// The config file as written (defaults when it is missing or unreadable); doctor never creates it.
pub fn read_config(paths: &Paths) -> Config {
    std::fs::read_to_string(paths.config_path())
        .ok()
        .and_then(|t| Config::parse(&t).ok())
        .unwrap_or_default()
}

// ---------- files ----------

/// The environment the daemon runs with: a `PATH` that finds the agents and eye, the eye home when
/// it is not `~/.agent-eye`, and what each agent asks for.
fn daemon_env(o: &Opts, agents: &Agents) -> Vec<(String, String)> {
    let mut path_dirs: Vec<String> = vec![];
    for dir in agents.iter().filter_map(|a| a.executable_dir()) {
        path_dirs.push(dir.to_string_lossy().into_owned());
    }
    path_dirs.push(o.bin_dir.to_string_lossy().into_owned());
    path_dirs.extend(
        crate::platform::system_path_dirs()
            .iter()
            .map(|d| d.to_string()),
    );
    let mut seen = HashSet::new();
    path_dirs.retain(|d| seen.insert(d.clone()));
    let mut env = vec![("PATH".to_string(), path_dirs.join(":"))];
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if home.as_ref().map(|h| h.join(".agent-eye")) != Some(o.eye_home.clone()) {
        env.push((
            "EYE_HOME".to_string(),
            o.eye_home.to_string_lossy().into_owned(),
        ));
    }
    env.extend(agents.iter().flat_map(|a| a.daemon_env()));
    env
}

/// The service file: the launch agent's plist or the systemd unit.
pub fn unit_text(o: &Opts, agents: &Agents) -> String {
    o.manager.unit_text(&Unit {
        bin: &o.eye_bin(),
        logs: &o.eye_home.join("logs"),
        env: &daemon_env(o, agents),
    })
}

// ---------- install / uninstall ----------

/// Copy the binary into place and link the `eye` alias next to it; the row says what changed.
fn install_binary(o: &Opts) -> Result<Row> {
    let dry = o.dry_run;
    let same_exe = same_target(&o.exe, &o.eye_bin());
    let owned = o.package.map(|p| format!("{}'s", p.name()));
    let state = if same_exe || same_file(&o.exe, &o.eye_bin()) {
        owned.as_deref().unwrap_or("unchanged")
    } else {
        let existed = o.eye_bin().exists();
        if !dry {
            let bytes = std::fs::read(&o.exe)?;
            write_file(&o.eye_bin(), &bytes, 0o755)?;
        }
        match (dry, existed) {
            (false, false) => "installed",
            (false, true) => "updated",
            (true, false) => "would install",
            (true, true) => "would update",
        }
    };
    // the `eye` alias is a symlink next to the binary (it replaces an old copy of the binary); one
    // that already runs this binary, like Homebrew's own link, stays as it is
    let alias_ok = std::fs::read_link(o.alias_bin()).is_ok_and(|t| t == Path::new(BIN_NAME))
        || same_target(&o.alias_bin(), &o.eye_bin());
    if !dry && !alias_ok {
        let a = o.alias_bin();
        if a.symlink_metadata().is_ok() {
            std::fs::remove_file(&a)?;
        }
        std::os::unix::fs::symlink(BIN_NAME, &a)?;
    }
    let link = match (alias_ok, dry) {
        (true, _) => "",
        (false, false) => " (linked)",
        (false, true) => " (would link)",
    };
    let mut text = format!(
        "{} ({state}), {ALIAS_NAME} -> {BIN_NAME}{link}",
        show_path(&o.eye_bin())
    );
    if let Some(note) = relink_old_copy(o)? {
        text.push_str(&format!("; {note}"));
    }
    Ok(Row::new("binary", text))
}

/// A copy an earlier install left in `~/.local/bin` becomes a link to the package manager's binary:
/// sessions already running keep calling the path their hooks had, and a `PATH` that finds that
/// folder first must not run an old version.
fn relink_old_copy(o: &Opts) -> Result<Option<String>> {
    let Some(dir) = &o.old_bin_dir else {
        return Ok(None);
    };
    let old = dir.join(BIN_NAME);
    if !old
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_file())
    {
        return Ok(None);
    }
    if !o.dry_run {
        let tmp = dir.join(format!(".{BIN_NAME}.link"));
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(o.eye_bin(), &tmp)?;
        std::fs::rename(&tmp, &old)?;
    }
    let verb = if o.dry_run { "would link" } else { "now links" };
    Ok(Some(format!(
        "the older copy {} {verb} to it",
        show_path(&old)
    )))
}

/// Do these two paths end at the same file, after every link?
fn same_target(a: &Path, b: &Path) -> bool {
    matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(x), Ok(y)) if x == y
    )
}

/// Are these two files the same bytes?
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    ma.len() == mb.len() && matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// Turn the board on when asked, and say how it stands.
fn board_row(o: &Opts) -> Result<Row> {
    let cfg_path = o.eye_home.join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|t| Config::parse(&t).ok())
        .unwrap_or_default();
    let turning_on = o.board && !cfg.board.enabled;
    if turning_on && !o.dry_run {
        crate::config::write_board_enabled(&cfg_path, true)?;
    }
    let text = if o.board || cfg.board.enabled {
        let lead = match (turning_on, o.dry_run) {
            (true, true) => "would turn on",
            (true, false) => "turned on",
            (false, _) => "on",
        };
        format!(
            "{lead}: http://127.0.0.1:{}/ (`{} board` prints the link with its token)",
            cfg.board.port,
            crate::pick::bin_name()
        )
    } else {
        format!(
            "off (turn it on with `{} board on`)",
            crate::pick::bin_name()
        )
    };
    Ok(Row::new("board", text))
}

/// Install the binary, the alias, each agent's hooks and skill, and the background service.
/// Nothing is written when an agent says its files are not in a state eye can edit safely.
pub fn install(o: &Opts, agents: &Agents) -> Result<Report> {
    let mut rep = Report::default();
    let dry = o.dry_run;
    let eye_bin = o.eye_bin();
    let ctx = InstallCtx {
        eye_bin: &eye_bin,
        dry_run: dry,
    };
    for a in agents.iter() {
        a.preflight(&ctx)?;
    }

    rep.push(install_binary(o)?);
    for a in agents.iter() {
        rep.extend(a.install(&ctx)?);
    }

    // the background service
    let unit = unit_text(o, agents);
    let existed = o.unit().exists();
    let unit_same = std::fs::read_to_string(o.unit()).is_ok_and(|t| t == unit);
    let state = match (existed, unit_same) {
        (_, true) => FileState::Unchanged,
        (false, false) => FileState::Added,
        (true, false) => FileState::Updated,
    };
    if !dry {
        std::fs::create_dir_all(o.eye_home.join("logs"))?;
        if !unit_same {
            write_file(&o.unit(), unit.as_bytes(), 0o644)?;
        }
    }
    // a service of an older install is stopped, and waited for, before the new one is loaded
    let mut retired = false;
    let mut older = vec![];
    for (file, old) in o.old_units() {
        if dry {
            older.push(format!("would remove the older {}", show_path(&file)));
            continue;
        }
        if let Some(path) = o.service_path() {
            let run = Runner {
                path: &path,
                timing: o.timing,
            };
            match o.manager.retire(&run, &old) {
                Ok(was_loaded) => retired |= was_loaded,
                Err(why) => {
                    older.push(format!("could not stop the older {} ({why})", old.name));
                    rep.failed = true;
                    continue;
                }
            }
        }
        std::fs::remove_file(&file)?;
        older.push(format!("removed the older {}", show_path(&file)));
    }
    let said = service::start(o, !unit_same || retired, state);
    rep.failed |= !said.ok;
    let mut text = said.text;
    for note in older {
        text.push_str(&format!("; {note}"));
    }
    rep.push(Row::new(service::AREA, text));

    rep.push(board_row(o)?);
    Ok(rep)
}

/// Remove what `install` added. `agents` should be every known agent, enabled or not: hooks stay
/// behind in an agent's files when it is switched off after installing.
pub fn uninstall(o: &Opts, agents: &Agents) -> Result<Report> {
    let mut rep = Report::default();
    let dry = o.dry_run;
    let remove = if dry { "would remove" } else { "removed" };
    if o.purge {
        // A mistyped EYE_HOME must not turn --purge into "delete my home folder".
        let holds_home = [account_home(), std::env::var_os("HOME").map(PathBuf::from)]
            .into_iter()
            .flatten()
            .any(|h| h.starts_with(&o.eye_home));
        anyhow::ensure!(
            !holds_home,
            "refusing to delete {}: it holds your home folder, so it is not eye's data folder",
            o.eye_home.display()
        );
    }

    // the daemon: stopped, and waited for, before its files go
    let said = service::stop(o);
    rep.failed |= !said.ok;
    let mut text = vec![said.text];
    let old_units = o.old_units();
    let mut kept = vec![];
    if let Some(path) = o.service_path() {
        let run = Runner {
            path: &path,
            timing: o.timing,
        };
        for (file, old) in &old_units {
            if let Err(why) = o.manager.retire(&run, old) {
                text.push(format!("could not stop the older {} ({why})", old.name));
                rep.failed = true;
                kept.push(file.clone());
            }
        }
    }
    let what = o.manager.what();
    let mut gone = vec![];
    for (name, p) in std::iter::once((what.to_string(), o.unit())).chain(
        old_units
            .into_iter()
            .filter(|(f, _)| !kept.contains(f))
            .map(|(f, _)| (format!("older {what}"), f)),
    ) {
        if p.symlink_metadata().is_ok() {
            if !dry {
                std::fs::remove_file(&p)?;
            }
            gone.push(format!("{remove} {}", show_path(&p)));
        } else if name == what {
            gone.push(format!("{what} file not there"));
        }
    }
    if let Some(path) = o.service_path() {
        let run = Runner {
            path: &path,
            timing: o.timing,
        };
        // harmless when it fails: systemd forgets the removed unit at its next reload
        let _ = o.manager.forget(&run);
    }
    text.extend(gone);
    rep.push(Row::new(service::AREA, text.join(", ")));

    // the binary and the alias. A package manager's binary is its own to remove, and so is
    // Homebrew's `eye` link; the links eye made go.
    let owner = package::owner(&o.eye_bin(), package::cargo_home().as_deref()).map(|(p, _)| p);
    let mut present = vec![];
    if owner.is_none() {
        present.push(o.eye_bin());
    }
    if owner != Some(Package::Homebrew) {
        present.push(o.alias_bin());
    }
    if let Some(dir) = &o.old_bin_dir {
        let old = dir.join(BIN_NAME);
        if std::fs::read_link(&old).is_ok_and(|t| t == o.eye_bin()) {
            present.push(old);
            let alias = dir.join(ALIAS_NAME);
            if std::fs::read_link(&alias).is_ok_and(|t| t == Path::new(BIN_NAME)) {
                present.push(alias);
            }
        }
    }
    present.retain(|p| p.symlink_metadata().is_ok());
    for p in &present {
        if !dry {
            std::fs::remove_file(p)?;
        }
    }
    // each path in full, then only the name of one in the same folder as the one before it
    let mut names = vec![];
    for (i, p) in present.iter().enumerate() {
        let same_dir = i > 0 && present[i - 1].parent() == p.parent();
        names.push(match p.file_name().filter(|_| same_dir) {
            Some(n) => n.to_string_lossy().into_owned(),
            None => show_path(p),
        });
    }
    let mut binary = vec![];
    if let Some(p) = owner {
        binary.push(format!(
            "kept {} (`{}` removes it)",
            show_path(&o.eye_bin()),
            p.remove_hint()
        ));
    }
    if !names.is_empty() {
        binary.push(format!("{remove} {}", names.join(", ")));
    }
    if binary.is_empty() {
        binary.push(format!("not there ({})", show_path(&o.eye_bin())));
    }
    rep.push(Row::new("binary", binary.join("; ")));

    let eye_bin = o.eye_bin();
    let ctx = InstallCtx {
        eye_bin: &eye_bin,
        dry_run: dry,
    };
    for a in agents.iter() {
        rep.extend(a.uninstall(&ctx)?);
    }
    rep.push(Row::new(
        "data",
        if o.purge {
            if !dry && o.eye_home.exists() {
                std::fs::remove_dir_all(&o.eye_home)?;
            }
            format!(
                "{} {}",
                if dry { "would delete" } else { "deleted" },
                show_path(&o.eye_home)
            )
        } else {
            format!("kept {} (use --purge to delete it)", show_path(&o.eye_home))
        },
    ));
    Ok(rep)
}

// ---------- doctor ----------

/// Is eye on `PATH`, as `agent-eye` or as the `eye` alias? Either is enough to run it.
fn path_check(bin_dir: &Path, path: Option<&std::ffi::OsStr>) -> Check {
    let find = |name: &str| path.and_then(|p| find_in(name, p));
    match (find(BIN_NAME), find(ALIAS_NAME)) {
        (Some(p), Some(_)) => Check::ok(format!(
            "{BIN_NAME} and {ALIAS_NAME} are on PATH ({})",
            p.display()
        )),
        (Some(p), None) => Check::ok(format!(
            "{BIN_NAME} is on PATH ({}); {ALIAS_NAME} is not",
            p.display()
        )),
        (None, Some(p)) => Check::ok(format!(
            "{ALIAS_NAME} is on PATH ({}); {BIN_NAME} is not",
            p.display()
        )),
        (None, None) => Check::warn(format!(
            "neither {BIN_NAME} nor {ALIAS_NAME} is on PATH (add {} to PATH)",
            bin_dir.display()
        )),
    }
}

/// Is the service manager running the service? Asked only with a search path (see `service_path`)
/// and when the service file is there; a manager with no way to ask is not asked.
fn service_check(o: &Opts, unit_there: bool, path: Option<&OsStr>) -> Option<Check> {
    let step = o.manager.status()?;
    let (active, state) = step.read(path.filter(|_| unit_there)?)?;
    Some(if active {
        Check::ok(format!("the {} is active", o.manager.what()))
    } else {
        Check::warn(format!(
            "the {} is {} (checked with `{step}`)",
            o.manager.what(),
            if state.is_empty() {
                "not reachable"
            } else {
                &state
            }
        ))
    })
}

/// Checks that everything eye relies on is in place: the binary, the service, the daemon, the
/// hooks of each enabled agent and whether the files they write still parse. Changes nothing.
pub fn doctor(o: &Opts, paths: &Paths, cfg: &Config, agents: &Agents) -> Vec<Check> {
    let mut out: Vec<Check> = vec![];
    out.push(path_check(&o.bin_dir, std::env::var_os("PATH").as_deref()));
    let unit = o.unit().exists();
    out.push(Check {
        ok: unit,
        msg: if unit {
            format!("{} file present ({})", o.manager.what(), o.unit().display())
        } else {
            format!("{} file missing ({})", o.manager.what(), o.unit().display())
        },
    });
    match daemon::running_pid(paths) {
        Some(pid) => out.push(Check::ok(format!("daemon alive (pid {pid})"))),
        None => out.push(Check::warn(format!(
            "daemon not running ({} daemon, or {})",
            crate::pick::bin_name(),
            o.manager.start_hint()
        ))),
    }
    out.extend(service_check(o, unit, o.service_path().as_deref()));
    let board = &cfg.board;
    out.push(Check::ok(if board.enabled {
        format!("board: on at http://127.0.0.1:{}", board.port)
    } else {
        format!(
            "board: off (optional; `{} board on`)",
            crate::pick::bin_name()
        )
    }));
    out.push(Check::ok(crate::remind::notices::describe(
        crate::remind::notices::is_on(paths, cfg.remind.notify),
        crate::remind::notices::available(),
    )));
    let eye_bin = o.eye_bin();
    let ctx = InstallCtx {
        eye_bin: &eye_bin,
        dry_run: true,
    };
    if agents.is_empty() {
        out.push(Check::warn(
            "no agent is enabled, so there is nothing to follow (see [agents] enabled in config.toml)",
        ));
    }
    for id in agents::unknown_ids(cfg) {
        out.push(Check::warn(format!(
            "[agents] enabled names '{id}', which this build does not know"
        )));
    }
    for a in agents.iter() {
        out.push(Check::ok(format!("agent: {} ({})", a.name(), a.id())));
        out.extend(a.doctor(&ctx));
    }
    if let Some(a) = agents.summary_agent(cfg) {
        if cfg.summary.enabled {
            out.push(Check::ok(format!("summaries are written by {}", a.name())));
        }
    } else if cfg.summary.enabled && !cfg.summary.agent.trim().is_empty() {
        out.push(Check::warn(format!(
            "[summary] agent is '{}', which is not enabled; no summaries are written",
            cfg.summary.agent
        )));
    }
    let last = crate::db::Db::open(&paths.db_path())
        .ok()
        .and_then(|d| d.meta_get("last_hook_ts").ok().flatten())
        .and_then(|v| v.parse::<i64>().ok());
    match last {
        Some(t) => {
            let age = (now_ms() - t) / 1000;
            out.push(Check {
                ok: age < 24 * 3600,
                msg: format!("last hook event {}", ago(now_ms(), t)),
            });
        }
        None => out.push(Check::warn("no hook event received yet")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use crate::platform::test_support::{self, FAST};
    use serde_json::{json, Value};
    use std::sync::Arc;

    const RTK_LIKE: &str = r#"{
  "env": {
    "A": "1"
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "/Users/x/.claude/hooks/rtk-rewrite.sh"
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/opt/notify/bin/notify stop"
          }
        ]
      }
    ]
  },
  "permissions": {
    "allow": [
      "Bash(git status:*)"
    ]
  }
}
"#;

    const MANAGERS: [Manager; 2] = [Manager::Launchd, Manager::Systemd];

    /// The text of the daemon's row.
    fn daemon(r: &Report) -> &str {
        &r.row("daemon").expect("a daemon row").text
    }

    fn opts_for(t: &Path, manager: Manager) -> Opts {
        Opts {
            eye_home: t.join("home"),
            bin_dir: t.join("bin"),
            package: None,
            old_bin_dir: None,
            manager,
            service_dir: t.join("agents"),
            exe: std::env::current_exe().unwrap(),
            dry_run: false,
            no_service: true,
            purge: false,
            board: false,
            programs: None,
            timing: Timing::REAL,
        }
    }

    /// Options for the service manager of this OS.
    fn opts(t: &Path) -> Opts {
        opts_for(t, Manager::current())
    }

    /// Claude Code under `t/claude`, which exists.
    fn claude(t: &Path) -> (Agents, PathBuf) {
        let dir = t.join("claude");
        std::fs::create_dir_all(&dir).unwrap();
        (
            Agents::from_vec(vec![Arc::new(Claude::new(dir.clone()))]),
            dir.join("settings.json"),
        )
    }

    #[test]
    fn only_the_daemon_of_this_home_is_restarted() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let o = opts_for(t.path(), manager);
            assert!(!o.unit_serves_home(), "{manager:?}: no unit, no daemon");
            std::fs::create_dir_all(&o.service_dir).unwrap();
            std::fs::write(o.unit(), unit_text(&o, &Agents::default())).unwrap();
            assert!(o.unit_serves_home(), "{manager:?}");
            let mut other = o.clone();
            other.eye_home = t.path().join("other");
            assert!(
                !other.unit_serves_home(),
                "{manager:?}: the unit runs another home"
            );
        }
    }

    #[test]
    fn a_temp_service_dir_never_runs_the_service_manager() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let mut o = opts_for(t.path(), manager);
            o.no_service = false;
            assert!(o.service_path().is_none(), "{manager:?}");
            let r = restart_daemon(&o);
            assert!(!r.failed);
            assert_eq!(
                daemon(&r),
                "not restarted (the service folder is not the default one); restart the daemon to apply"
            );
        }
    }

    #[test]
    fn the_real_service_dir_runs_it_unless_told_not_to() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let mut o = opts_for(t.path(), manager);
            o.service_dir = manager.default_dir(&account_home().unwrap());
            o.no_service = false;
            assert!(o.service_path().is_some(), "{manager:?}");
            o.no_service = true;
            assert!(o.service_path().is_none(), "--no-service");
            o.no_service = false;
            o.dry_run = true;
            assert!(o.service_path().is_none(), "--dry-run");
        }
    }

    #[test]
    fn round_trip_keeps_other_entries_byte_identical() {
        for manager in MANAGERS {
            round_trip(manager);
        }
    }

    fn round_trip(manager: Manager) {
        let t = tempfile::tempdir().unwrap();
        let o = opts_for(t.path(), manager);
        let (agents, settings) = claude(t.path());
        std::fs::write(&settings, RTK_LIKE).unwrap();

        let mut dry = o.clone();
        dry.dry_run = true;
        let rep = install(&dry, &agents).unwrap().body(true);
        assert!(rep.contains("would add hooks (8 events) to "), "{rep}");
        assert!(rep.contains("\"SessionStart\""), "the diff is there: {rep}");
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            RTK_LIKE,
            "dry-run writes nothing"
        );
        assert!(!o.bin_dir.exists() && !o.service_dir.exists());

        install(&o, &agents).unwrap();
        let after = std::fs::read_to_string(&settings).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        for ev in ["SessionStart", "Stop", "SessionEnd", "Notification"] {
            let groups = v["hooks"][ev].as_array().unwrap();
            let ours: Vec<_> = groups
                .iter()
                .filter(|g| g.to_string().contains("eye hook"))
                .collect();
            assert_eq!(ours.len(), 1, "{ev}");
            assert_eq!(ours[0]["hooks"][0]["async"], true);
            assert_eq!(ours[0]["hooks"][0]["timeout"], 5);
            // the form already on installed machines: no agent argument
            assert!(ours[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .ends_with(&format!("agent-eye hook {ev}")));
        }
        assert!(after.contains("rtk-rewrite.sh") && after.contains("/opt/notify/bin/notify stop"));
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        let skill = t.path().join("claude/skills/eye/SKILL.md");
        assert!(
            std::fs::read_link(o.alias_bin()).unwrap() == Path::new(BIN_NAME)
                && o.eye_bin().is_file()
                && o.unit().is_file()
                && skill.is_file()
        );
        let unit = std::fs::read_to_string(o.unit()).unwrap();
        match manager {
            Manager::Launchd => {
                assert!(unit.contains("<string>daemon</string>") && unit.contains("KeepAlive"))
            }
            Manager::Systemd => {
                assert!(unit.contains("agent-eye daemon\n") && unit.contains("Restart=on-failure"))
            }
        }
        // Claude Code lives in a non-default folder, so the daemon is told where
        assert!(unit.contains("EYE_CLAUDE_DIR"));
        assert!(std::fs::read_dir(t.path().join("claude"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("settings.json.eye-backup-")));

        // second install changes nothing
        let again = install(&o, &agents).unwrap().body(true);
        assert!(again.contains("hooks ok (8 events)"), "{again}");
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), after);

        uninstall(&o, &agents).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), RTK_LIKE);
        assert!(
            o.alias_bin().symlink_metadata().is_err()
                && !o.eye_bin().exists()
                && !o.unit().exists()
                && !skill.exists()
        );
        assert!(!t.path().join("claude/skills/eye").exists());
    }

    /// Options for a Homebrew install under `t/brew`, run the way `Opts::resolve` sets it up, with
    /// an older copy of the binary (and its alias) in `t/local`.
    fn homebrew_opts(t: &Path) -> Opts {
        let prefix = package::tests::brew(&t.join("brew"), "0.2.0");
        let local = t.join("local");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join(BIN_NAME), "an older copy").unwrap();
        std::os::unix::fs::symlink(BIN_NAME, local.join(ALIAS_NAME)).unwrap();
        let mut o = opts(t);
        o.exe = prefix.join("bin/agent-eye");
        o.bin_dir = prefix.join("opt/agent-eye/bin");
        o.package = Some(Package::Homebrew);
        o.old_bin_dir = Some(local);
        o
    }

    #[test]
    fn a_homebrew_binary_runs_where_it_is_and_an_older_copy_becomes_a_link_to_it() {
        let t = tempfile::tempdir().unwrap();
        let o = homebrew_opts(t.path());
        let keg = o.bin_dir.join(BIN_NAME).canonicalize().unwrap();
        let rep = install(&o, &Agents::default()).unwrap();
        let row = &rep.row("binary").unwrap().text;
        assert!(
            row.contains("agent-eye (Homebrew's), eye -> agent-eye;"),
            "{row}"
        );
        assert!(row.contains("now links to it"), "{row}");
        // Homebrew's files are as brew left them
        assert_eq!(std::fs::read_to_string(&keg).unwrap(), "0.2.0");
        assert_eq!(
            std::fs::read_link(keg.with_file_name(ALIAS_NAME)).unwrap(),
            Path::new(BIN_NAME)
        );
        // the older copy is now the current binary, and the service runs the one that upgrades
        let local = o.old_bin_dir.clone().unwrap();
        assert_eq!(
            std::fs::read_link(local.join(BIN_NAME)).unwrap(),
            o.eye_bin()
        );
        assert_eq!(
            std::fs::read_to_string(local.join(ALIAS_NAME)).unwrap(),
            "0.2.0"
        );
        let unit = std::fs::read_to_string(o.unit()).unwrap();
        assert!(unit.contains(&o.eye_bin().display().to_string()), "{unit}");

        let again = install(&o, &Agents::default()).unwrap();
        assert!(!again.row("binary").unwrap().text.contains("links"));

        let gone = uninstall(&o, &Agents::default()).unwrap();
        let row = &gone.row("binary").unwrap().text;
        assert!(
            row.contains("(`brew uninstall agent-eye` removes it); removed "),
            "{row}"
        );
        assert!(keg.is_file() && keg.with_file_name(ALIAS_NAME).exists());
        assert!(local.join(BIN_NAME).symlink_metadata().is_err());
        assert!(local.join(ALIAS_NAME).symlink_metadata().is_err());
    }

    #[test]
    fn a_dry_run_says_it_would_link_the_older_copy_and_leaves_it() {
        let t = tempfile::tempdir().unwrap();
        let mut o = homebrew_opts(t.path());
        o.dry_run = true;
        let rep = install(&o, &Agents::default()).unwrap();
        assert!(rep.row("binary").unwrap().text.contains("would link to it"));
        let old = o.old_bin_dir.unwrap().join(BIN_NAME);
        assert_eq!(std::fs::read_to_string(old).unwrap(), "an older copy");
    }

    #[test]
    fn an_eye_that_already_runs_the_binary_is_left_as_it_is() {
        // `--bin-dir /opt/homebrew/bin`: brew's own `eye` link must survive, or the next
        // `brew upgrade` cannot link the new version
        let t = tempfile::tempdir().unwrap();
        let prefix = package::tests::brew(t.path(), "0.2.0");
        let mut o = opts(t.path());
        o.exe = prefix.join("bin/agent-eye");
        o.bin_dir = prefix.join("bin");
        let brews = std::fs::read_link(o.alias_bin()).unwrap();
        let rep = install(&o, &Agents::default()).unwrap();
        let row = &rep.row("binary").unwrap().text;
        assert!(!row.contains("linked"), "{row}");
        assert_eq!(std::fs::read_link(o.alias_bin()).unwrap(), brews);

        let gone = uninstall(&o, &Agents::default()).unwrap();
        assert!(gone.row("binary").unwrap().text.starts_with("kept "));
        assert_eq!(std::fs::read_link(o.alias_bin()).unwrap(), brews);
        assert!(o.eye_bin().exists());
    }

    #[test]
    fn purge_refuses_a_folder_that_holds_the_home_folder() {
        let t = tempfile::tempdir().unwrap();
        let (agents, _) = claude(t.path());
        for bad in [PathBuf::from("/"), account_home().unwrap()] {
            let mut o = opts(t.path());
            o.eye_home = bad;
            o.purge = true;
            o.dry_run = true;
            assert!(uninstall(&o, &agents).is_err());
        }
        let mut o = opts(t.path());
        o.purge = true;
        assert!(uninstall(&o, &agents).is_ok());
    }

    #[test]
    fn install_creates_missing_settings_and_uninstall_empties_it() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let (agents, settings) = claude(t.path());
        install(&o, &agents).unwrap();
        assert!(settings.exists());
        uninstall(&o, &agents).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v, json!({}));
    }

    /// A settings file somebody formatted by hand: one line, tabs and a key order of their own.
    const HAND_FORMATTED: &str = "{\"permissions\":{\"allow\":[\"Bash(git status:*)\"]},\n\t\"hooks\": {\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"/opt/notify/bin/notify stop\"}]}]},\n  \"env\":{\"A\":\"1\"}}";

    fn backups(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings.json.eye-backup-")
            })
            .collect()
    }

    #[test]
    fn a_hand_formatted_settings_file_is_reformatted_after_a_backup() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let (agents, settings) = claude(t.path());
        std::fs::write(&settings, HAND_FORMATTED).unwrap();
        let original: Value = serde_json::from_str(HAND_FORMATTED).unwrap();

        let mut dry = o.clone();
        dry.dry_run = true;
        let rep = install(&dry, &agents).unwrap().body(true);
        assert!(
            rep.contains("would be rewritten as 2-space JSON, after a backup"),
            "{rep}"
        );
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), HAND_FORMATTED);

        let rep = install(&o, &agents).unwrap();
        let row = rep.row("claude").unwrap();
        assert!(
            row.text.contains("backup ") && row.text.contains("rewritten as 2-space JSON"),
            "one line says so: {}",
            row.text
        );
        // the diff shows what eye added, not the whole file
        assert!(!row.detail.iter().any(|l| l.starts_with("- ")), "{row:?}");
        let saved = backups(&t.path().join("claude"));
        assert_eq!(saved.len(), 1);
        assert_eq!(std::fs::read_to_string(&saved[0]).unwrap(), HAND_FORMATTED);
        let after = std::fs::read_to_string(&settings).unwrap();
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(after, crate::files::pretty_json(&v), "now 2-space JSON");
        assert_eq!(v["permissions"], original["permissions"]);
        assert_eq!(v["env"], original["env"]);
        assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(v["hooks"]["Stop"][0].to_string().contains("notify stop"));

        // hooks already there: the file is not touched again, not even to reformat it
        std::fs::write(&settings, after.replace("\n  ", "\n    ")).unwrap();
        let again = install(&o, &agents).unwrap().body(true);
        assert!(again.contains("hooks ok (8 events)") && !again.contains("rewritten"));
        assert_eq!(backups(&t.path().join("claude")).len(), 1);

        // uninstall takes out eye's hooks and nothing else
        uninstall(&o, &agents).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v, original);
    }

    #[test]
    fn uninstall_reformats_a_hand_formatted_file_and_keeps_its_entries() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let (agents, settings) = claude(t.path());
        std::fs::write(&settings, HAND_FORMATTED).unwrap();
        install(&o, &agents).unwrap();
        let mut v: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        v["theme"] = json!("dark");
        std::fs::write(&settings, serde_json::to_string(&v).unwrap()).unwrap();
        let rep = uninstall(&o, &agents).unwrap().body(true);
        assert!(
            rep.contains("backup ") && rep.contains("rewritten as 2-space JSON"),
            "{rep}"
        );
        let back: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        let mut want: Value = serde_json::from_str(HAND_FORMATTED).unwrap();
        want["theme"] = json!("dark");
        assert_eq!(back, want);
    }

    #[test]
    fn bad_json_and_a_wrong_shape_stop_the_install_before_anything_is_written() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let (agents, settings) = claude(t.path());
        for bad in ["{nope", "[]", "{\"hooks\": {\"Stop\": \"x\"}}"] {
            std::fs::write(&settings, bad).unwrap();
            assert!(install(&o, &agents).is_err(), "{bad}");
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), bad);
            assert!(!o.bin_dir.exists() && !o.service_dir.exists(), "{bad}");
        }
    }

    #[test]
    fn nothing_installed_for_an_agent_that_is_not_there() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        // Claude Code has no folder here and no program: skipped, and no folder is made
        let dir = t.path().join("no-claude");
        let agents = Agents::from_vec(vec![Arc::new(
            Claude::new(dir.clone()).with_executable("/nonexistent/claude"),
        )]);
        let rep = install(&o, &agents).unwrap();
        let row = &rep.row("claude").unwrap().text;
        assert!(row.starts_with("not found (no "), "{row}");
        assert!(row.contains("no `claude` on PATH"), "{row}");
        assert!(
            !dir.exists(),
            "no folder is made for an agent that is not installed"
        );
        assert!(o.unit().is_file());
        // no agent at all: still installs, with no row for an agent
        let none = install(&o, &Agents::default()).unwrap();
        assert!(!none.failed && none.row("claude").is_none());
        assert!(none.row("binary").is_some() && none.row("daemon").is_some());
    }

    #[test]
    fn doctor_finds_eye_on_path_under_either_name() {
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let search = std::env::join_paths([&bin]).unwrap();
        let check = |p: Option<&std::ffi::OsStr>| path_check(&t.path().join("home-bin"), p);
        // neither: the one warning, naming both
        let c = check(Some(&search));
        assert!(
            !c.ok && c.msg.contains("neither agent-eye nor eye"),
            "{}",
            c.msg
        );
        assert!(!check(None).ok);
        // only the alias (what a user who runs `eye` has): fine, and says what is missing
        std::fs::write(bin.join(ALIAS_NAME), "").unwrap();
        let c = check(Some(&search));
        assert!(c.ok && c.msg.starts_with("eye is on PATH") && c.msg.ends_with("agent-eye is not"));
        // only the long name
        std::fs::remove_file(bin.join(ALIAS_NAME)).unwrap();
        std::fs::write(bin.join(BIN_NAME), "").unwrap();
        let c = check(Some(&search));
        assert!(c.ok && c.msg.starts_with("agent-eye is on PATH") && c.msg.ends_with("eye is not"));
        // both
        std::fs::write(bin.join(ALIAS_NAME), "").unwrap();
        let c = check(Some(&search));
        assert!(
            c.ok && c.msg.starts_with("agent-eye and eye are on PATH"),
            "{}",
            c.msg
        );
    }

    /// Options for a service manager that has a stand-in for its program (`t/stub`): the real
    /// manager is never run.
    fn with_stand_in(t: &Path, manager: Manager) -> (Opts, PathBuf) {
        let stub = t.join("stub");
        std::fs::create_dir_all(&stub).unwrap();
        match manager {
            Manager::Launchd => test_support::launchd(&stub),
            Manager::Systemd => test_support::systemd(&stub),
        }
        let mut o = opts_for(t, manager);
        o.no_service = false;
        o.programs = Some(stub.clone().into_os_string());
        o.timing = FAST;
        (o, stub)
    }

    /// The state file that says the stand-in's service is running.
    fn running(manager: Manager) -> &'static str {
        match manager {
            Manager::Launchd => "loaded",
            Manager::Systemd => "active",
        }
    }

    fn program(manager: Manager) -> &'static str {
        match manager {
            Manager::Launchd => "launchctl",
            Manager::Systemd => "systemctl",
        }
    }

    /// What the stand-in was asked since the last time, and forget it.
    fn asked(stub: &Path, manager: Manager) -> Vec<String> {
        let name = program(manager);
        let got = test_support::calls(stub, name);
        let _ = std::fs::remove_file(stub.join(format!("{name}.log")));
        got
    }

    #[test]
    fn an_install_starts_a_daemon_that_is_not_running_and_restarts_one_that_is() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let (o, stub) = with_stand_in(t.path(), manager);
            let none = Agents::default();
            let run = |o: &Opts| install(o, &none).unwrap();

            let first = run(&o);
            let text = daemon(&first);
            assert!(!first.failed, "{manager:?}: {text}");
            assert!(text.starts_with("started ("), "{manager:?}: {text}");
            assert!(text.contains(", file added"), "{manager:?}: {text}");
            assert!(has_pid(text) == (manager == Manager::Launchd), "{text}");
            let want: &[&str] = match manager {
                Manager::Launchd => &["print", "bootstrap", "print"],
                Manager::Systemd => &["is-active", "daemon-reload", "enable", "is-active"],
            };
            assert_eq!(asked(&stub, manager), want, "{manager:?}: not running");

            // running, file as it was: restarted in place, never unloaded
            let again = run(&o);
            let text = daemon(&again);
            assert!(!again.failed, "{text}");
            assert!(
                text.starts_with("restarted on the new binary ("),
                "{manager:?}: {text}"
            );
            let want: &[&str] = match manager {
                Manager::Launchd => &["print", "kickstart", "print"],
                Manager::Systemd => &["is-active", "restart", "is-active"],
            };
            assert_eq!(
                asked(&stub, manager),
                want,
                "{manager:?}: running, unchanged"
            );

            // running, the file changes: loaded again from the new one
            let mut text = std::fs::read_to_string(o.unit()).unwrap();
            text.push('\n');
            std::fs::write(o.unit(), text).unwrap();
            let changed = run(&o);
            assert!(!changed.failed, "{:?}", changed.rows);
            assert!(
                daemon(&changed).starts_with("restarted with its new "),
                "{manager:?}: {:?}",
                changed.rows
            );
            let calls = asked(&stub, manager);
            match manager {
                Manager::Launchd => {
                    let at = |w: &str| calls.iter().position(|c| c == w).unwrap();
                    assert!(at("bootout") < at("bootstrap"), "{calls:?}");
                    assert_eq!(calls.iter().filter(|c| *c == "bootstrap").count(), 1);
                    assert!(!calls.contains(&"kickstart".into()), "{calls:?}");
                }
                Manager::Systemd => assert_eq!(
                    calls,
                    [
                        "is-active",
                        "daemon-reload",
                        "enable",
                        "restart",
                        "is-active"
                    ]
                ),
            }
            assert!(test_support::has(&stub, running(manager)));
        }
    }

    fn has_pid(text: &str) -> bool {
        text.contains(", pid 4242)")
    }

    #[test]
    fn an_install_that_cannot_start_the_daemon_says_so_and_fails() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let (o, stub) = with_stand_in(t.path(), manager);
            match manager {
                Manager::Launchd => test_support::set(&stub, "bootstrap_fails", "99"),
                Manager::Systemd => test_support::set(&stub, "no_session", ""),
            }
            let done = install(&o, &Agents::default()).unwrap();
            let line = daemon(&done);
            assert!(done.failed, "{manager:?}: {line}");
            assert!(
                line.starts_with("could not start (")
                    && line.ends_with("doctor`, or start it with `agent-eye daemon`."),
                "{line}"
            );
            match manager {
                Manager::Launchd => assert!(
                    line.contains("(launchctl bootstrap failed: Input/output error)"),
                    "{line}"
                ),
                Manager::Systemd => assert!(line.contains("Failed to connect to bus"), "{line}"),
            }
            // the manager's own words are not passed on
            let all = done.body(true);
            for raw in ["Try re-running", "Bootstrap failed: 5"] {
                assert!(!all.contains(raw), "{raw}: {all}");
            }
            assert!(o.unit().is_file(), "the file is written all the same");
        }
    }

    #[test]
    fn an_uninstall_waits_until_the_service_is_gone_before_it_removes_anything() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let (o, stub) = with_stand_in(t.path(), manager);
            install(&o, &Agents::default()).unwrap();
            assert!(test_support::has(&stub, running(manager)));
            asked(&stub, manager);
            test_support::set(&stub, "unloading", "3");
            let done = uninstall(&o, &Agents::default()).unwrap();
            let text = daemon(&done);
            assert!(!done.failed, "{text}");
            assert!(text.starts_with("stopped ("), "{manager:?}: {text}");
            assert!(text.contains(", removed "), "{manager:?}: {text}");
            assert!(!test_support::has(&stub, running(manager)), "{manager:?}");
            assert!(!o.unit().exists());
            let calls = asked(&stub, manager);
            match manager {
                Manager::Launchd => {
                    let out = calls.iter().position(|c| c == "bootout").unwrap();
                    let looks = calls[out..].iter().filter(|c| *c == "print").count();
                    assert!(
                        looks >= 3,
                        "it looked until the service was gone: {calls:?}"
                    );
                }
                Manager::Systemd => assert_eq!(
                    calls,
                    ["is-active", "disable", "daemon-reload"],
                    "{manager:?}"
                ),
            }
            // nothing running: said, and not an error
            let again = uninstall(&o, &Agents::default()).unwrap();
            assert!(!again.failed);
            assert!(daemon(&again).starts_with("was not running"));
        }
    }

    #[test]
    fn an_uninstall_that_cannot_stop_the_daemon_says_so_and_fails() {
        let t = tempfile::tempdir().unwrap();
        let (o, stub) = with_stand_in(t.path(), Manager::Launchd);
        install(&o, &Agents::default()).unwrap();
        test_support::set(&stub, "unloading", "100000");
        let done = uninstall(&o, &Agents::default()).unwrap();
        let text = daemon(&done);
        assert!(done.failed, "{text}");
        assert!(
            text.starts_with("could not stop (launchctl bootout did not finish"),
            "{text}"
        );
        assert!(text.contains("`launchctl bootout gui/"), "by hand: {text}");
    }

    #[test]
    fn an_older_launch_agent_is_stopped_waited_for_and_removed_before_the_new_one_starts() {
        let t = tempfile::tempdir().unwrap();
        let (o, stub) = with_stand_in(t.path(), Manager::Launchd);
        let old = o.service_dir.join("dev.claude-eye.daemon.plist");
        std::fs::create_dir_all(&o.service_dir).unwrap();
        std::fs::write(&old, "old").unwrap();
        test_support::set(&stub, "loaded", "");
        test_support::set(&stub, "unloading", "3");
        let done = install(&o, &Agents::default()).unwrap();
        assert!(!done.failed, "{:?}", done.rows);
        assert!(!old.exists());
        assert!(
            daemon(&done).ends_with("; removed the older dev.claude-eye.daemon.plist")
                || daemon(&done).contains("; removed the older "),
            "{}",
            daemon(&done)
        );
        let calls = asked(&stub, Manager::Launchd);
        let at = |w: &str| calls.iter().position(|c| c == w).unwrap();
        assert!(at("bootout") < at("bootstrap"), "{calls:?}");
        assert_eq!(
            calls.iter().filter(|c| *c == "bootstrap").count(),
            1,
            "no bootstrap came before the old one was gone: {calls:?}"
        );
        // a unit has no older service
        assert!(opts_for(t.path(), Manager::Systemd).old_units().is_empty());
    }

    #[test]
    fn without_permission_to_run_the_manager_the_commands_are_said() {
        let t = tempfile::tempdir().unwrap();
        let (agents, _) = claude(t.path());
        let mut o = opts_for(t.path(), Manager::Systemd);
        o.dry_run = true;
        let rep = install(&o, &agents).unwrap();
        let unit = o.unit().display().to_string();
        assert!(unit.ends_with("agents/agent-eye.service"), "{unit}");
        assert_eq!(
            daemon(&rep),
            format!("would add {unit}, then start the daemon (systemd unit agent-eye.service)")
        );
        let rep = uninstall(&o, &agents).unwrap();
        assert_eq!(
            daemon(&rep),
            "would stop (systemd unit agent-eye.service), systemd unit file not there"
        );
        assert!(!o.service_dir.exists(), "dry runs write nothing");

        // --no-service: what to run by hand, for each manager
        o.dry_run = false;
        let rep = install(&o, &agents).unwrap();
        assert_eq!(
            daemon(&rep),
            format!("added {unit}; not started (--no-service). To start it run `systemctl --user daemon-reload`, then `systemctl --user enable --now agent-eye.service`")
        );
        let rep = install(&o, &agents).unwrap();
        assert!(
            daemon(&rep).starts_with(&format!("{unit} unchanged; not started (--no-service)")),
            "{}",
            daemon(&rep)
        );
        let mut o = opts_for(t.path(), Manager::Launchd);
        o.dry_run = true;
        let rep = uninstall(&o, &agents).unwrap();
        assert_eq!(
            daemon(&rep),
            "would stop (launch agent dev.agent-eye.daemon), launch agent file not there"
        );
        o.dry_run = false;
        let rep = uninstall(&o, &agents).unwrap();
        assert!(
            daemon(&rep)
                .starts_with("not stopped (--no-service). To stop it run `launchctl bootout gui/"),
            "{}",
            daemon(&rep)
        );
        // a service folder that is not the real one is not run either, and the line says why
        o.no_service = false;
        let rep = install(&o, &agents).unwrap();
        assert!(
            daemon(&rep).contains(
                "; not started (the service folder is not the default one). To start it run `launchctl bootstrap gui/"
            ),
            "{}",
            daemon(&rep)
        );
    }

    #[test]
    fn board_on_and_off_start_or_restart_the_daemon_and_say_when_they_cannot() {
        for manager in MANAGERS {
            let t = tempfile::tempdir().unwrap();
            let (o, stub) = with_stand_in(t.path(), manager);
            // no service file for this home: nothing to restart
            let r = restart_daemon(&o);
            assert!(!r.failed);
            assert_eq!(
                daemon(&r),
                format!(
                    "not restarted (no {} file for this eye home); restart the daemon to apply",
                    manager.what()
                )
            );
            std::fs::create_dir_all(&o.service_dir).unwrap();
            std::fs::write(o.unit(), unit_text(&o, &Agents::default())).unwrap();
            let r = restart_daemon(&o);
            assert!(
                !r.failed && daemon(&r).starts_with("started ("),
                "{manager:?}: {}",
                daemon(&r)
            );
            asked(&stub, manager);
            let r = restart_daemon(&o);
            assert!(
                !r.failed && daemon(&r).starts_with("restarted ("),
                "{manager:?}: {}",
                daemon(&r)
            );
            assert!(!daemon(&r).contains("new binary"), "{}", daemon(&r));
            let want: &[&str] = match manager {
                Manager::Launchd => &["print", "kickstart", "print"],
                Manager::Systemd => &["is-active", "restart", "is-active"],
            };
            assert_eq!(asked(&stub, manager), want);
            match manager {
                Manager::Launchd => test_support::set(&stub, "kickstart_fails", ""),
                Manager::Systemd => test_support::set(&stub, "no_session", ""),
            }
            test_support::set(&stub, "bootstrap_fails", "99");
            let r = restart_daemon(&o);
            assert!(
                r.failed && daemon(&r).starts_with("could not restart ("),
                "{manager:?}: {}",
                daemon(&r)
            );
        }
    }

    #[test]
    fn an_older_launch_agent_is_removed_on_install_and_a_unit_has_none() {
        let t = tempfile::tempdir().unwrap();
        let o = opts_for(t.path(), Manager::Launchd);
        let old = o.service_dir.join("dev.claude-eye.daemon.plist");
        std::fs::create_dir_all(&o.service_dir).unwrap();
        std::fs::write(&old, "old").unwrap();
        let rep = install(&o, &Agents::default()).unwrap();
        assert!(
            daemon(&rep).ends_with("; removed the older dev.claude-eye.daemon.plist")
                || daemon(&rep).contains("; removed the older "),
            "{}",
            daemon(&rep)
        );
        assert!(!old.exists());
        let o = opts_for(t.path(), Manager::Systemd);
        assert!(o.old_units().is_empty());
    }

    #[test]
    fn the_daemon_gets_a_path_that_finds_the_agent_and_eye() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let (agents, _) = claude(t.path());
        let env = daemon_env(&o, &agents);
        assert_eq!(env[0].0, "PATH");
        let dirs: Vec<&str> = env[0].1.split(':').collect();
        let bin = o.bin_dir.to_string_lossy().into_owned();
        assert!(dirs.contains(&bin.as_str()), "{dirs:?}");
        assert!(
            dirs.contains(&"/usr/bin") && dirs.contains(&"/bin"),
            "{dirs:?}"
        );
        assert_eq!(
            dirs.contains(&"/opt/homebrew/bin"),
            cfg!(target_os = "macos"),
            "Homebrew's folder on a Mac"
        );
        let unique: HashSet<_> = dirs.iter().collect();
        assert_eq!(unique.len(), dirs.len(), "no folder twice");
        // the eye home is named because it is not ~/.agent-eye; Claude Code's folder because it is not ~/.claude
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert!(
            keys.contains(&"EYE_HOME") && keys.contains(&"EYE_CLAUDE_DIR"),
            "{keys:?}"
        );
    }

    #[test]
    fn doctor_says_whether_the_service_is_active() {
        let t = tempfile::tempdir().unwrap();
        let o = opts_for(t.path(), Manager::Systemd);
        let ask =
            |path: Option<&Path>, there: bool| service_check(&o, there, path.map(Path::as_os_str));
        crate::platform::test_support::script(t.path(), "systemctl", "echo active");
        let c = ask(Some(t.path()), true).unwrap();
        assert!(c.ok && c.msg == "the systemd unit is active", "{}", c.msg);
        crate::platform::test_support::script(t.path(), "systemctl", "echo inactive\nexit 3");
        let c = ask(Some(t.path()), true).unwrap();
        assert!(
            !c.ok
                && c.msg.contains("is inactive")
                && c.msg
                    .contains("systemctl --user is-active agent-eye.service"),
            "{}",
            c.msg
        );
        crate::platform::test_support::script(t.path(), "systemctl", "exit 1");
        assert!(ask(Some(t.path()), true)
            .unwrap()
            .msg
            .contains("not reachable"));
        assert!(
            ask(Some(t.path()), false).is_none(),
            "no unit file, nothing to ask"
        );
        assert!(ask(None, true).is_none(), "not allowed to run it");
        let empty = tempfile::tempdir().unwrap();
        assert!(ask(Some(empty.path()), true).is_none(), "no systemctl here");
        let launchd = opts_for(t.path(), Manager::Launchd);
        assert!(service_check(&launchd, true, Some(t.path().as_os_str())).is_none());
    }

    #[test]
    fn doctor_names_the_service_file_of_its_manager() {
        for (manager, what) in [
            (Manager::Launchd, "launch agent"),
            (Manager::Systemd, "systemd unit"),
        ] {
            let t = tempfile::tempdir().unwrap();
            let o = opts_for(t.path(), manager);
            let paths = Paths {
                home: o.eye_home.clone(),
            };
            let r = doctor(&o, &paths, &Config::default(), &Agents::default());
            assert!(
                r.iter()
                    .any(|c| !c.ok && c.msg.starts_with(&format!("{what} file missing"))),
                "{manager:?}"
            );
            assert!(
                r.iter().any(|c| c.msg.contains("daemon not running")
                    && c.msg.contains(manager.start_hint())),
                "{manager:?}"
            );
        }
    }

    #[test]
    fn doctor_says_whether_due_reminders_show_a_desktop_notice() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let paths = Paths {
            home: o.eye_home.clone(),
        };
        let (agents, _) = claude(t.path());
        let line = |cfg: &Config| {
            let r = doctor(&o, &paths, cfg, &agents);
            let l: Vec<_> = r
                .iter()
                .filter(|c| c.msg.contains("desktop notices"))
                .collect();
            assert_eq!(l.len(), 1);
            l[0].msg.clone()
        };
        assert!(line(&Config::default()).contains("off"));
        let mut on = Config::default();
        on.remind.notify = true;
        assert!(line(&on).contains(": on"));
    }

    #[test]
    fn doctor_runs_on_empty_dirs() {
        let t = tempfile::tempdir().unwrap();
        let o = opts(t.path());
        let paths = Paths {
            home: o.eye_home.clone(),
        };
        let (agents, _) = claude(t.path());
        let r = doctor(&o, &paths, &Config::default(), &agents);
        assert!(r.iter().any(|c| !c.ok && c.msg.contains("hooks missing")));
        // no agent enabled: a clear line, no crash
        let none = doctor(&o, &paths, &Config::default(), &Agents::default());
        assert!(none
            .iter()
            .any(|c| !c.ok && c.msg.contains("no agent is enabled")));
    }
}
