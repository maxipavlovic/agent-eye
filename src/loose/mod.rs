//! Loose ends: what a session left behind. Servers and shells still running, containers, git
//! worktrees in a scratchpad, commits on no remote, modified files nobody committed.
//!
//! Finding them only reads (`find`). Acting (`act`) happens only on a confirmed request that names
//! one stored item, and never pushes, commits, force-removes or kills with SIGKILL.

mod act;
mod find;
mod gitscan;
mod probe;
mod store;
#[cfg(test)]
mod tests;

pub use act::{copy_command, remove, send_prompt, stop};
pub use find::{find, Input, Sess};
pub use probe::{Docker, Probe, System};
pub use store::{counts, get, ignore, list, ports_of, replace, summary_line};

use crate::agents::Agents;
use crate::config::Config;
use crate::db::Db;
use anyhow::Result;
use serde_json::{json, Value};

/// What kind of leftover an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Process,
    Container,
    Worktree,
    Unpushed,
    Uncommitted,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Process => "process",
            Kind::Container => "container",
            Kind::Worktree => "worktree",
            Kind::Unpushed => "unpushed",
            Kind::Uncommitted => "uncommitted",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "process" => Kind::Process,
            "container" => Kind::Container,
            "worktree" => Kind::Worktree,
            "unpushed" => Kind::Unpushed,
            "uncommitted" => Kind::Uncommitted,
            _ => return None,
        })
    }

    /// What eye can do about it: stop, remove, or (for git work it never touches) copy a command.
    pub fn action(self) -> &'static str {
        match self {
            Kind::Process | Kind::Container => "stop",
            Kind::Worktree => "remove",
            Kind::Unpushed => "push",
            Kind::Uncommitted => "commit",
        }
    }
}

/// One thing a session left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LooseEnd {
    /// Stable across scans: `proc:<pid>:<start>`, `box:<id>:<created>`, `worktree:<path>`,
    /// `unpushed:<repo>:<newest commit>`, `uncommitted:<repo>`.
    pub key: String,
    pub session_id: String,
    pub kind: Kind,
    /// One plain sentence.
    pub line: String,
    /// First line: what it is, as the confirm question names it. The rest: more to read.
    pub detail: String,
    pub since_ms: i64,
}

impl LooseEnd {
    pub fn action(&self) -> &'static str {
        self.kind.action()
    }

    /// What the item is, in a few words (the first line of `detail`).
    pub fn what(&self) -> &str {
        self.detail.lines().next().unwrap_or("")
    }

    /// The question asked before acting; None when eye never acts on this kind.
    pub fn confirm(&self) -> Option<String> {
        match self.kind {
            Kind::Process | Kind::Container => Some(format!("Stop {}?", self.what())),
            Kind::Worktree => Some(format!("Remove worktree {}?", self.what())),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "key": self.key, "session_id": self.session_id, "kind": self.kind.as_str(),
            "line": self.line, "detail": self.detail, "what": self.what(),
            "since_ms": self.since_ms, "action": self.action(),
            "confirm": self.confirm(), "command": copy_command(self),
        })
    }
}

/// A scan: every loose end of the sessions active lately, with the real machine.
pub fn scan(db: &Db, cfg: &Config, agents: &Agents, now: i64) -> Result<Vec<LooseEnd>> {
    let input = find::load(db, cfg, agents, now)?;
    Ok(find(
        &input,
        &System::with_roots(agents.scratch_roots()),
        now,
    ))
}

/// Scan and store: what `eye loose` and the daemon do. Does nothing when `[loose]` is off.
pub fn scan_and_store(db: &Db, cfg: &Config, agents: &Agents, now: i64) -> Result<usize> {
    if !cfg.loose.enabled {
        return Ok(0);
    }
    let items = scan(db, cfg, agents, now)?;
    replace(db, &items, now)?;
    Ok(items.len())
}
