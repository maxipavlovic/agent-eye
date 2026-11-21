//! Loose ends in git: worktrees in a scratchpad or whose folder is gone, commits on no remote,
//! modified tracked files. Every call is read only (`Probe::git`), with a timeout.

use super::find::{norm, Index};
use super::{Kind, LooseEnd, Probe};
use crate::util::home_rel;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Git calls that run side by side.
const THREADS: usize = 8;
/// Commits and files listed in an item's detail.
const SHOWN: usize = 5;

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Worktree {
    pub path: PathBuf,
    pub prunable: bool,
    pub bare: bool,
}

pub(super) fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut out = vec![];
    for block in text.split("\n\n") {
        let mut path = None;
        let (mut prunable, mut bare) = (false, false);
        for l in block.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if l == "bare" {
                bare = true;
            } else if l == "prunable" || l.starts_with("prunable ") {
                prunable = true;
            }
        }
        if let Some(path) = path {
            out.push(Worktree {
                path,
                prunable,
                bare,
            });
        }
    }
    out
}

/// `abc1234<TAB>1700000000<TAB>subject` lines of the unpushed log.
pub(super) fn parse_unpushed(text: &str) -> Vec<(String, i64, String)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let sha = it.next()?.to_string();
            let ts = it.next()?.parse().ok()?;
            Some((sha, ts, it.next().unwrap_or("").to_string()))
        })
        .filter(|(sha, _, _)| !sha.is_empty())
        .collect()
}

/// Modified tracked files out of `git status --porcelain` (untracked `??` are left out).
pub(super) fn parse_modified(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.len() > 3 && !l.starts_with("??") && !l.starts_with("!!"))
        .map(|l| l[3..].to_string())
        .collect()
}

fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|sc| {
        for _ in 0..THREADS.min(items.len()) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { break };
                let r = f(item);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// One repo (all its worktrees) and the sessions that worked in it.
struct Group {
    main: PathBuf,
    worktrees: Vec<Worktree>,
    /// The folders sessions used, each with the sessions that did.
    paths: BTreeMap<PathBuf, Vec<usize>>,
    sessions: Vec<usize>,
}

pub(super) fn find(idx: &Index, probe: &dyn Probe, now: i64) -> Vec<LooseEnd> {
    let _ = now;
    let input = idx.input;
    // the folder of each session's repo
    let mut by_path: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (i, s) in input.sessions.iter().enumerate() {
        let Some(dir) = s.repo_root.as_deref().or(s.cwd.as_deref()) else {
            continue;
        };
        let p = PathBuf::from(dir);
        if p.is_absolute() && idx.usable(&p) && p.is_dir() {
            by_path.entry(p).or_default().push(i);
        }
    }
    let paths: Vec<(&PathBuf, &Vec<usize>)> = by_path.iter().collect();
    let listed = par_map(&paths, |(p, _)| {
        probe
            .git(p, &["worktree", "list", "--porcelain"])
            .map(|t| parse_worktrees(&t))
    });
    let mut groups: BTreeMap<PathBuf, Group> = BTreeMap::new();
    for ((path, sessions), wts) in paths.iter().zip(listed) {
        let Some(wts) = wts.filter(|w| !w.is_empty()) else {
            continue;
        };
        let main = norm(&wts[0].path);
        let g = groups.entry(main.clone()).or_insert_with(|| Group {
            main,
            worktrees: wts,
            paths: BTreeMap::new(),
            sessions: vec![],
        });
        g.paths.insert((*path).clone(), (*sessions).clone());
        g.sessions.extend(sessions.iter().copied());
    }
    let groups: Vec<Group> = groups.into_values().collect();
    par_map(&groups, |g| group_items(idx, probe, g))
        .into_iter()
        .flatten()
        .collect()
}

/// The item for commits on no remote, only where the repo has a remote at all.
fn unpushed_item(
    probe: &dyn Probe,
    at: &Path,
    g: &Group,
    repo: &str,
    sid: &str,
) -> Option<LooseEnd> {
    probe
        .git(at, &["remote"])
        .filter(|t| !t.trim().is_empty())?;
    let log = probe.git(
        at,
        &[
            "log",
            "--branches",
            "--not",
            "--remotes",
            "--format=%h%x09%ct%x09%s",
        ],
    )?;
    let commits = parse_unpushed(&log);
    let (newest, _, _) = commits.first()?;
    let n = commits.len();
    let oldest = commits.iter().map(|c| c.1).min().unwrap_or(0) * 1000;
    let mut detail = format!("{} in {repo}", plural(n, "commit", "commits"));
    for (sha, _, subject) in commits.iter().take(SHOWN) {
        detail.push_str(&format!("\n{sha} {}", crate::util::head_chars(subject, 80)));
    }
    if n > SHOWN {
        detail.push_str(&format!("\n+{} more", n - SHOWN));
    }
    Some(LooseEnd {
        key: format!("unpushed:{}:{newest}", g.main.display()),
        session_id: sid.to_string(),
        kind: Kind::Unpushed,
        line: format!(
            "{} in {repo} {} on no remote",
            plural(n, "commit", "commits"),
            if n == 1 { "is" } else { "are" }
        ),
        detail,
        since_ms: oldest,
    })
}

fn group_items(idx: &Index, probe: &dyn Probe, g: &Group) -> Vec<LooseEnd> {
    let sessions = &idx.input.sessions;
    // the most recently active session of the repo owns what is left, while it is not open
    let Some(&owner) = g.sessions.iter().max_by_key(|&&i| sessions[i].last) else {
        return vec![];
    };
    let repo = g
        .main
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = vec![];

    // worktrees in a scratchpad, or whose folder is gone
    for wt in g.worktrees.iter().skip(1).filter(|w| !w.bare) {
        let scratch = idx.in_scratch(&wt.path);
        if !scratch && !wt.prunable {
            continue;
        }
        let who = idx.by_session_id(&wt.path).unwrap_or(owner);
        if sessions[who].live {
            continue;
        }
        let shown = home_rel(&wt.path.to_string_lossy());
        let (line, why) = if wt.prunable {
            (
                format!("worktree {shown} of {repo} points to a folder that is gone"),
                "folder gone",
            )
        } else {
            (
                format!("worktree {shown} of {repo} sits in a scratchpad"),
                "scratchpad",
            )
        };
        out.push(LooseEnd {
            key: format!("worktree:{}", wt.path.display()),
            session_id: sessions[who].id.clone(),
            kind: Kind::Worktree,
            line,
            detail: format!("{shown}\nrepo {}\n{why}", g.main.display()),
            since_ms: sessions[who].last,
        });
    }
    if sessions[owner].live {
        return out;
    }
    let sid = &sessions[owner].id;
    let at: &Path = if g.main.is_dir() {
        &g.main
    } else {
        match g.paths.keys().next() {
            Some(p) => p,
            None => return out,
        }
    };

    // the log of commits and the status of each folder run side by side
    // modified tracked files, per folder the sessions used
    let dirs: Vec<&PathBuf> = g.paths.keys().collect();
    let (unpushed, statuses) = std::thread::scope(|sc| {
        let log = sc.spawn(|| unpushed_item(probe, at, g, &repo, sid));
        let statuses = par_map(&dirs, |d| {
            probe
                .git(d, &["status", "--porcelain", "-uno"])
                .map(|t| parse_modified(&t))
        });
        (log.join().ok().flatten(), statuses)
    });
    out.extend(unpushed);
    for (dir, files) in dirs.iter().zip(statuses) {
        let Some(files) = files.filter(|f| !f.is_empty()) else {
            continue;
        };
        let n = files.len();
        let shown = home_rel(&dir.to_string_lossy());
        let mut detail = format!(
            "{} in {shown}",
            plural(n, "modified file", "modified files")
        );
        for f in files.iter().take(SHOWN) {
            detail.push_str(&format!("\n{f}"));
        }
        if n > SHOWN {
            detail.push_str(&format!("\n+{} more", n - SHOWN));
        }
        out.push(LooseEnd {
            key: format!("uncommitted:{}", dir.display()),
            session_id: sid.clone(),
            kind: Kind::Uncommitted,
            line: format!(
                "{} in {repo} {} not committed",
                plural(n, "modified file", "modified files"),
                if n == 1 { "is" } else { "are" }
            ),
            detail,
            since_ms: sessions[owner].last,
        });
    }
    out
}
