//! Acting on one stored item, after a confirmed request. Stop sends SIGTERM to a process whose
//! pid and start time still match, or `docker stop` to a container; remove runs
//! `git worktree remove` without `--force`. Nothing here pushes, commits or kills harder.

use super::store::{forget, get};
use super::{Kind, LooseEnd, Probe};
use crate::db::Db;
use crate::util::{printable, shell_quote};
use anyhow::{anyhow, bail, Result};
use std::path::Path;

fn item(db: &Db, key: &str, kinds: &[Kind]) -> Result<LooseEnd> {
    let it = get(db, key)?.ok_or_else(|| anyhow!("no such leftover: {key} (scan again)"))?;
    if !kinds.contains(&it.kind) {
        bail!(
            "{} cannot do that to a {}",
            crate::pick::bin_name(),
            it.kind.as_str()
        );
    }
    Ok(it)
}

/// `proc:<pid>:<start>` as its two numbers.
fn proc_key(key: &str) -> Option<(u32, i64)> {
    let rest = key.strip_prefix("proc:")?;
    let (pid, start) = rest.split_once(':')?;
    Some((pid.parse().ok()?, start.parse().ok()?))
}

/// `box:<id>:<created>`.
fn box_key(key: &str) -> Option<(&str, i64)> {
    let (id, created) = key.strip_prefix("box:")?.split_once(':')?;
    Some((id, created.parse().ok()?))
}

/// Stop a process or a container. Returns what was done, in a sentence.
pub fn stop(db: &Db, probe: &dyn Probe, key: &str) -> Result<String> {
    let it = item(db, key, &[Kind::Process, Kind::Container])?;
    let what = it.what().to_string();
    match it.kind {
        Kind::Process => {
            let (pid, start) = proc_key(key).ok_or_else(|| anyhow!("not a process key"))?;
            let Some(now) = probe.proc_info(pid) else {
                forget(db, key)?;
                bail!("{what} is not running any more; nothing was stopped");
            };
            if now.start != start {
                forget(db, key)?;
                bail!(
                    "pid {pid} now belongs to another process (it started at a different time); nothing was stopped"
                );
            }
            if now.uid != crate::platform::uid() {
                bail!("pid {pid} is not yours; nothing was stopped");
            }
            probe
                .terminate(pid)
                .map_err(|e| anyhow!("could not stop pid {pid}: {e}"))?;
            forget(db, key)?;
            Ok(format!("sent SIGTERM to {what}"))
        }
        _ => {
            let (id, created) = box_key(key).ok_or_else(|| anyhow!("not a container key"))?;
            let alive = probe
                .docker()
                .into_iter()
                .any(|c| c.id == id && c.created == created);
            if !alive {
                forget(db, key)?;
                bail!("{what} is not running any more; nothing was stopped");
            }
            probe.docker_stop(id).map_err(|e| anyhow!("{e}"))?;
            forget(db, key)?;
            Ok(format!("stopped {what}"))
        }
    }
}

/// Remove a worktree with plain `git worktree remove`: git refuses when it has changes, and what
/// it says is passed on as it is. A record whose folder is gone is not pruned (that would touch the
/// whole repo): eye drops its item and says what to run.
pub fn remove(db: &Db, probe: &dyn Probe, key: &str) -> Result<String> {
    let it = item(db, key, &[Kind::Worktree])?;
    let path = key
        .strip_prefix("worktree:")
        .ok_or_else(|| anyhow!("not a worktree key"))?;
    let lines: Vec<&str> = it.detail.lines().collect();
    let repo = lines
        .iter()
        .find_map(|l| l.strip_prefix("repo "))
        .ok_or_else(|| anyhow!("the item does not name its repo (scan again)"))?;
    let repo = Path::new(repo);
    // git must still list it, and it must not be the main worktree
    let listed = probe
        .git(repo, &["worktree", "list", "--porcelain"])
        .map(|t| super::gitscan::parse_worktrees(&t))
        .unwrap_or_default();
    if !listed.iter().skip(1).any(|w| w.path == Path::new(path)) {
        forget(db, key)?;
        bail!("git no longer lists that worktree; nothing was removed");
    }
    if !Path::new(path).exists() {
        // `git worktree prune` works on the whole repo and would drop records of other worktrees
        // too (one on an unmounted disk): eye only drops its own item and says what to run
        forget(db, key)?;
        return Ok(format!(
            "the folder is gone; `git worktree prune` in {} cleans git's record",
            repo.display()
        ));
    }
    let said = probe
        .git_act(repo, &["worktree", "remove", "--", path])
        .map_err(|e| anyhow!("{e}"))?;
    forget(db, key)?;
    Ok(if said.is_empty() {
        format!("removed worktree {}", it.what())
    } else {
        said
    })
}

/// The command to copy for an item eye never runs itself: push the commits, commit the files.
pub fn copy_command(it: &LooseEnd) -> Option<String> {
    match it.kind {
        Kind::Unpushed => {
            let rest = it.key.strip_prefix("unpushed:")?;
            let (repo, _) = rest.rsplit_once(':')?;
            Some(format!("cd {} && git push --all", shell_quote(repo)))
        }
        Kind::Uncommitted => {
            let dir = it.key.strip_prefix("uncommitted:")?;
            Some(format!("cd {} && git commit -a", shell_quote(dir)))
        }
        _ => None,
    }
}

/// The first message of a session that is sent back: what it left, one line, plain text.
pub fn send_prompt(items: &[LooseEnd]) -> String {
    let mut lines: Vec<String> = items.iter().take(8).map(|i| printable(&i.line)).collect();
    if items.len() > 8 {
        lines.push(format!("and {} more", items.len() - 8));
    }
    format!(
        "Before you call this finished, please deal with what this session left behind: {}. \
         Stop servers and containers you started, remove scratch worktrees, and tell me about any commits \
         not pushed or files not committed instead of pushing anything.",
        lines.join("; ")
    )
}
