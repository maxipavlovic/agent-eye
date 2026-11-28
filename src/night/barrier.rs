//! The barrier between a Claude night run and the folders the user works in.
//!
//! A continued session remembers the absolute paths of its original folder, and a sentence in the
//! prompt does not stop it from editing there. So every Claude night run also gets, through
//! `--settings`:
//!
//! * Claude Code's sandbox for Bash, with writes limited to the run's own folder, the temp folder
//!   and the few git folders a commit in a worktree has to write. `failIfUnavailable` makes Claude
//!   refuse to start rather than run unsandboxed, and `allowUnsandboxedCommands = false` removes
//!   the retry outside the sandbox.
//! * `sandbox.network`: the package registries of `[night] network` and no git host, so a command
//!   that reaches a remote over HTTPS fails however it is written. `ssh` ignores the proxy, so it
//!   cannot connect either (Claude's docs), and `ssh` and `scp` are denied as well.
//! * `sandbox.filesystem.denyWrite` and `Edit` deny rules for the original folder, so a rule in the
//!   user's own settings that opens it (an `Edit(//Users/me/code/**)` allow, an `allowWrite`, an
//!   `additionalDirectories`) cannot win: deny is checked before allow in every scope.
//!
//! Reads are not blocked. In a worktree run the original `.git` has to stay writable for the
//! worktree's commits, so that folder is denied entry by entry, `.git` aside from `hooks`, `config`,
//! `HEAD` and `index` (a Bash command that builds a path some other way can still make a new
//! top-level file there: Claude auto-allows a worktree's main checkout). In a copy run, and for a
//! plain folder, the original is denied whole.

use crate::config::Config;
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What a run may write, and what it may not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Barrier {
    /// Writable for the sandbox: the run's folder and the git folders its commits write.
    pub allow_write: Vec<PathBuf>,
    /// Readable for the sandbox even where the user's own settings close folders (`denyRead`,
    /// or a home folder that is closed by default): the folder of `eye` links on the run's `PATH`
    /// and the binary they point to. Left out of the settings when empty.
    pub allow_read: Vec<PathBuf>,
    /// Not writable for the sandbox, whatever else allows it.
    pub deny_write: Vec<PathBuf>,
    /// `Edit` deny rules (they cover the Edit, Write and NotebookEdit tools; the sandbox reads
    /// them as write rules too).
    pub deny_edit: Vec<String>,
    /// Folders a hook refuses the file tools in, for what no rule names: a new file in the folder.
    pub guard: Vec<PathBuf>,
}

/// Both spellings of a path when they differ (`/tmp/x` and `/private/tmp/x` on macOS).
fn spellings(p: &Path) -> Vec<PathBuf> {
    let mut v = vec![p.to_path_buf()];
    if let Ok(c) = p.canonicalize() {
        if c != p {
            v.push(c);
        }
    }
    v
}

/// The git folder a worktree keeps its own files in, read from its `.git` pointer file.
fn worktree_gitdir(root: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(root.join(".git")).ok()?;
    let line = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?;
    Some(PathBuf::from(line.trim()))
}

/// Make the folders the sandbox is told about exist: a rule for a path that is missing may be
/// refused (Linux binds the folder).
fn ensure(dir: &Path) -> bool {
    std::fs::create_dir_all(dir).is_ok()
}

impl Barrier {
    /// Let the sandbox read `p`, in both spellings when it is reached through a link.
    pub fn allow_reading(&mut self, p: &Path) {
        for s in spellings(p) {
            if !self.allow_read.contains(&s) {
                self.allow_read.push(s);
            }
        }
    }
}

/// Work out the barrier of a run working in `root` (a worktree or a copy) and protecting
/// `protect` (the planned folder and, for a continued session, its original working folder).
///
/// `whole`: the run works in a separate copy, so every protected folder is locked completely, a
/// repo's `.git` too. Otherwise (a worktree) a repo's `.git` has to stay writable for its commits.
pub fn plan(root: &Path, protect: &[PathBuf], whole: bool) -> Barrier {
    let mut b = Barrier::default();
    for r in spellings(root) {
        b.allow_write.push(r);
    }
    // a worktree: its commits write the shared objects, its own branch and its own folder
    let mut common = None;
    if let Some(gd) = worktree_gitdir(root) {
        if let Some(c) = gd.parent().and_then(Path::parent) {
            common = Some(c.to_path_buf());
            let refs = c.join("refs/heads/night");
            let logs = c.join("logs/refs/heads/night");
            let mut dirs = vec![c.join("objects"), gd.clone()];
            if ensure(&refs) && ensure(&logs) {
                dirs.extend([refs, logs]);
            }
            for d in dirs {
                b.allow_write.extend(spellings(&d));
            }
        }
    }
    // the original working tree of that repo is protected too, when the run started in a sub-folder
    let mut roots: Vec<PathBuf> = protect.to_vec();
    if let Some(top) = common
        .as_ref()
        .filter(|c| c.file_name().is_some_and(|n| n == ".git"))
        .and_then(|c| c.parent())
    {
        roots.push(top.to_path_buf());
    }
    let mut seen: Vec<PathBuf> = vec![];
    for r in roots {
        for p in spellings(&r) {
            if p.as_os_str().is_empty() || p.parent().is_none() || seen.contains(&p) {
                continue;
            }
            seen.push(p.clone());
            if p.starts_with(root) {
                continue;
            }
            protect_folder(&mut b, &p, whole);
        }
    }
    b
}

fn protect_folder(b: &mut Barrier, p: &Path, whole: bool) {
    b.guard.push(p.to_path_buf());
    let git = p.join(".git");
    if whole || !git.is_dir() {
        b.deny_write.push(p.to_path_buf());
        b.deny_edit.push(edit_rule(p, true));
        return;
    }
    // A repo: the sandbox cannot deny the folder whole (a deny beats the allow for `.git`, and a
    // worktree's commits write there), nor can an Edit rule (the sandbox reads it as a write
    // rule). So: every entry but `.git`, and in `.git` what changes behaviour.
    if let Ok(rd) = std::fs::read_dir(p) {
        let mut names: Vec<_> = rd.flatten().collect();
        names.sort_by_key(std::fs::DirEntry::file_name);
        for e in names.iter().filter(|e| e.file_name() != ".git") {
            let dir = e.file_type().is_ok_and(|t| t.is_dir());
            b.deny_write.push(e.path());
            b.deny_edit.push(edit_rule(&e.path(), dir));
        }
    }
    for (name, dir) in [
        ("hooks", true),
        ("config", false),
        ("HEAD", false),
        ("index", false),
    ] {
        b.deny_write.push(git.join(name));
        b.deny_edit.push(edit_rule(&git.join(name), dir));
    }
}

/// An `Edit` rule for an absolute path: `//` is the filesystem root in permission rules.
fn edit_rule(p: &Path, dir: bool) -> String {
    let s = p.to_string_lossy();
    let s = s.trim_start_matches('/');
    if dir {
        format!("Edit(//{s}/**)")
    } else {
        format!("Edit(//{s})")
    }
}

/// `s` as a literal in a shell `case` pattern: every character that is not plain is escaped.
fn pat(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || "/_.-".contains(c) || !c.is_ascii() {
                vec![c]
            } else {
                vec!['\\', c]
            }
        })
        .collect()
}

/// The shell command of a PreToolUse hook that refuses a tool call naming a guarded folder. It
/// reads the call as JSON text on stdin and answers exit code 2, which blocks it, with a message
/// that says what is refused and what to do instead (the run is shown it). For the file tools
/// (`bash` false) the `file_path` or `notebook_path` must not be in a guarded folder or climb out
/// with `..`; for Bash the folder's path must not follow the command's key anywhere in the call,
/// so a `description` that names it is refused too. In the JSON a quote, newline or tab after the
/// path is `\"`, `\n` or `\t`, so a backslash ends a path as well. Reading is not refused: the
/// Read tool is not hooked, so the message sends a run there. That is a net for the mistake this
/// exists for, an agent using a remembered absolute path, not for a command that builds the path
/// some other way; the sandbox is what holds. A folder whose name has a quote or a backslash in
/// it is left to the rules.
pub fn hook_command(guard: &[PathBuf], bash: bool) -> Option<String> {
    let mut pats = vec![];
    let mut named = vec![];
    for g in guard {
        let t = g.to_string_lossy();
        if t.contains(['\'', '"', '\\', '$', '`', '\n']) {
            continue;
        }
        named.push(t.to_string());
        let t = t.trim_end_matches('/');
        if bash {
            for tail in ["/", " ", "\"", "'", ";", "&", "|", ")", "\\", ":"] {
                pats.push(format!("*command\\\":\\\"*{}*", pat(&format!("{t}{tail}"))));
            }
        } else {
            pats.push(format!("*_path\\\":\\\"{}/*", pat(t)));
        }
    }
    if pats.is_empty() {
        return None;
    }
    if !bash {
        pats.push("*_path\\\":\\\"*/../*".into());
        pats.push("*_path\\\":\\\"../*".into());
    }
    let folders = named.join(", ");
    let said = if bash {
        format!("night run: a shell call that names the original folder ({folders}), in its command or its description, is refused; it belongs to you, not to the run; work in the run folder, and read a file from the original with the file-reading tool (Read)")
    } else {
        format!("night run: the file tools may write only in the run folder, not in the original folder ({folders}) and not through a path that climbs out of the run folder with ..")
    };
    Some(format!(
        "in=$(cat); case \"$in\" in {}) echo '{said}' >&2; exit 2;; esac; exit 0",
        pats.join("|")
    ))
}

/// The `--settings` JSON of a run. `network` is the only list of hosts a sandboxed command may
/// reach (Claude Code: `sandbox.network.allowedDomains`; `strictAllowlist` denies the rest instead
/// of asking). `sandboxed` is false only when the platform has no sandbox and
/// `[night] allow_unsandboxed = true`: the Edit rules still apply, Bash is then not confined.
pub fn settings(b: &Barrier, deny: &[&str], network: &[String], sandboxed: bool) -> String {
    let paths = |v: &[PathBuf]| -> Vec<Value> {
        v.iter()
            .map(|p| Value::String(p.to_string_lossy().into_owned()))
            .collect()
    };
    let mut rules: Vec<String> = deny.iter().map(|s| (*s).to_string()).collect();
    rules.extend(b.deny_edit.iter().cloned());
    let mut v = json!({ "permissions": { "deny": rules } });
    let mut hooks = vec![];
    for (matcher, bash) in [("Write|Edit|MultiEdit|NotebookEdit", false), ("Bash", true)] {
        if let Some(cmd) = hook_command(&b.guard, bash) {
            hooks.push(json!({
                "matcher": matcher,
                "hooks": [{ "type": "command", "command": cmd }],
            }));
        }
    }
    if !hooks.is_empty() {
        v["hooks"] = json!({ "PreToolUse": hooks });
    }
    if sandboxed {
        let mut filesystem = json!({
            "allowWrite": paths(&b.allow_write),
            "denyWrite": paths(&b.deny_write),
        });
        if !b.allow_read.is_empty() {
            filesystem["allowRead"] = Value::Array(paths(&b.allow_read));
        }
        v["sandbox"] = json!({
            "enabled": true,
            "failIfUnavailable": true,
            "allowUnsandboxedCommands": false,
            "autoAllowBashIfSandboxed": true,
            "filesystem": filesystem,
            "network": {
                "allowedDomains": network,
                "deniedDomains": super::launch::NETWORK_DENIED,
                "strictAllowlist": true,
            },
        });
    }
    v.to_string()
}

/// Does this machine have what Claude Code's sandbox needs? macOS has Seatbelt built in; Linux
/// needs `bwrap` (bubblewrap) and `socat` on PATH; nothing else is supported.
pub fn available() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let path = crate::platform::search_path();
        let missing: Vec<&str> = ["bwrap", "socat"]
            .into_iter()
            .filter(|n| crate::util::find_in(n, &path).is_none())
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Claude Code's sandbox needs {} on PATH (on Debian or Ubuntu: apt-get install bubblewrap socat)",
                missing.join(" and ")
            ))
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err("Claude Code's sandbox is not available on this platform".into())
    }
}

/// Whether a Claude night run may start: the sandbox is there, or `allow_unsandboxed` says to go on
/// without it. Returns whether the run will be sandboxed.
pub fn gate(cfg: &Config) -> Result<bool> {
    gate_with(cfg, available())
}

/// Why an agy night run is not started by default. `agy --sandbox` runs commands in a "terminal
/// sandbox" and asks before one that would leave it, and a headless run answers such a request with a
/// denial; but whether the sandbox keeps writes inside the run's folder was not shown end to end, so
/// it is not used as the barrier. `[night] allow_unsandboxed = true` starts the run anyway.
pub const AGY_UNVERIFIED: &str = "agy's --sandbox is not known to keep a run inside its own folder (eye could not verify it), so an Antigravity night run could write outside it; set [night] allow_unsandboxed = true to run it anyway";

/// Whether an agy night run may start. Returns whether it is confined, which is never known: the
/// only way through is `allow_unsandboxed`.
pub fn agy_gate(cfg: &Config) -> Result<bool> {
    if cfg.night.allow_unsandboxed {
        Ok(false)
    } else {
        bail!("{AGY_UNVERIFIED}")
    }
}

/// Whether an OpenCode night run may start. OpenCode has no sandbox of its own: its permission
/// rules refuse `git push`, `gh`, `ssh` and the publish commands and edits outside the run's
/// folder, but a shell command can still write anywhere. So it runs only when `[night]
/// allow_unsandboxed = true` says so.
pub fn gate_unsandboxed(cfg: &Config) -> Result<()> {
    anyhow::ensure!(
        cfg.night.allow_unsandboxed,
        "OpenCode has no sandbox: its permission rules refuse git push, gh, ssh, the publish commands and edits outside the run's folder, but a shell command could still write anywhere. eye does not start an OpenCode night run unless [night] allow_unsandboxed = true"
    );
    Ok(())
}

/// Whether a night run of `agent` may start, whatever the agent: its own gate, or none for an
/// agent that confines itself. Returns whether the run will be confined. This is what decides
/// that a run switched to another agent can start at all.
pub fn gate_for(cfg: &Config, agent: &str) -> Result<bool> {
    match agent {
        crate::agents::LEGACY_ID => gate(cfg),
        "agy" => agy_gate(cfg),
        "opencode" => gate_unsandboxed(cfg).map(|()| false),
        _ => Ok(true),
    }
}

pub fn gate_with(cfg: &Config, avail: Result<(), String>) -> Result<bool> {
    match avail {
        Ok(()) => Ok(true),
        Err(_) if cfg.night.allow_unsandboxed => Ok(false),
        Err(why) => bail!(
            "{why}. A Claude night run without it could write outside its own folder, so eye does not start it; install it, or set [night] allow_unsandboxed = true to run without"
        ),
    }
}
