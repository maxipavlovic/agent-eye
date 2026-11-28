//! Where a run works: a git worktree on a new branch for a repo (mode `worktree`), a local clone on
//! a new branch (mode `copy`), a copy for a plain folder. Never the original folder. Also: what the run changed, and removing it all again on DISCARD.

use super::Mode;
use crate::loose::{Probe, System};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// A plain folder bigger than this is not copied: the run is refused when it is planned.
pub const COPY_LIMIT: u64 = 100 << 20;
/// Folders a copy leaves out.
pub const SKIP: [&str; 5] = ["node_modules", ".git", "target", ".venv", "dist"];

/// Where a run will work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Isolation {
    /// The folder to start the agent in.
    pub cwd: PathBuf,
    /// The worktree (or copy) root; the run's own folder, removed on DISCARD.
    pub root: PathBuf,
    pub branch: Option<String>,
    pub base_ref: Option<String>,
}

/// The root of the repo `dir` is in, if it is in one.
pub fn repo_root(p: &dyn Probe, dir: &Path) -> Option<PathBuf> {
    let out = p.git(dir, &["rev-parse", "--show-toplevel"])?;
    let t = out.trim();
    (!t.is_empty()).then(|| PathBuf::from(t))
}

/// Size of a folder in bytes, skipping what a copy skips and symlinks; stops once past `cap`.
pub fn plain_size(dir: &Path, cap: u64) -> u64 {
    fn walk(d: &Path, cap: u64, total: &mut u64) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            if *total > cap {
                return;
            }
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name();
            if ft.is_dir() {
                if !SKIP.iter().any(|s| name == *s) {
                    walk(&e.path(), cap, total);
                }
            } else if ft.is_file() {
                *total += e.metadata().map_or(0, |m| m.len());
            }
        }
    }
    let mut total = 0;
    walk(dir, cap, &mut total);
    total
}

/// Refuse a folder that is not in a repo and is too big to copy.
pub fn check_plain_size(dir: &Path) -> Result<()> {
    if repo_root(&System::new(), dir).is_some() {
        return Ok(());
    }
    let size = plain_size(dir, COPY_LIMIT);
    anyhow::ensure!(
        size <= COPY_LIMIT,
        "{} is not a git repo and is bigger than 100 MB (not counting node_modules, .git, target, .venv, dist), so eye will not copy it for a night run; put it in a repo or pick a smaller folder",
        dir.display()
    );
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let ft = e.file_type()?;
        let name = e.file_name();
        if ft.is_dir() {
            if !SKIP.iter().any(|s| name == *s) {
                copy_tree(&e.path(), &dst.join(&name))?;
            }
        } else if ft.is_file() {
            std::fs::copy(e.path(), dst.join(&name))?;
        }
        // symlinks are left out: one could point outside the folder
    }
    Ok(())
}

fn branch_free(p: &dyn Probe, repo: &Path, name: &str) -> bool {
    p.git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
    .is_none()
}

/// Make the run's own folder under `run_dir` (`wt` for a worktree, `work` for a copy).
pub fn prepare(
    p: &dyn Probe,
    run_dir: &Path,
    folder: &Path,
    night: &str,
    slug: &str,
    run_id: i64,
    mode: Mode,
) -> Result<Isolation> {
    std::fs::create_dir_all(run_dir)?;
    if let Some(repo) = repo_root(p, folder) {
        let base = p
            .git(&repo, &["rev-parse", "HEAD"])
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .context(
                "the repo has no commit yet: commit once so a run has something to start from",
            )?;
        let mut branch = format!("night/{}-{slug}", super::clock::compact(night));
        // a copy's branch meets the repo only when it is brought in, so it always carries the run
        // number: two copies planned with the same words never bring the same name
        if mode == Mode::Copy || !branch_free(p, &repo, &branch) {
            branch = format!("{branch}-{run_id}");
        }
        // a run started in a sub-folder of the repo starts in the same sub-folder of its folder
        let rel = folder
            .canonicalize()
            .ok()
            .zip(repo.canonicalize().ok())
            .and_then(|(f, r)| f.strip_prefix(r).ok().map(Path::to_path_buf))
            .unwrap_or_default();
        if mode == Mode::Copy {
            // hardlinked objects: fast and small; the base is what the repo's HEAD is when the run
            // starts
            let work = run_dir.join("work");
            p.git_act(
                run_dir,
                &[
                    "clone",
                    "--local",
                    "--no-checkout",
                    &repo.to_string_lossy(),
                    &work.to_string_lossy(),
                ],
            )
            .map_err(|e| anyhow::anyhow!("git clone failed: {e}"))?;
            let made = p
                .git_act(&work, &["checkout", "-q", "-b", &branch, &base])
                .and_then(|_| p.git_act(&work, &["remote", "remove", "origin"]));
            if let Err(e) = made {
                let _ = std::fs::remove_dir_all(&work);
                bail!("preparing the copy failed: {e}");
            }
            return Ok(Isolation {
                cwd: work.join(rel),
                root: work,
                branch: Some(branch),
                base_ref: Some(base),
            });
        }
        let wt = run_dir.join("wt");
        p.git_act(
            &repo,
            &[
                "worktree",
                "add",
                &wt.to_string_lossy(),
                "-b",
                &branch,
                "HEAD",
            ],
        )
        .map_err(|e| anyhow::anyhow!("git worktree add failed: {e}"))?;
        return Ok(Isolation {
            cwd: wt.join(rel),
            root: wt,
            branch: Some(branch),
            base_ref: Some(base),
        });
    }
    check_plain_size(folder)?;
    let work = run_dir.join("work");
    copy_tree(folder, &work).context("copying the folder")?;
    Ok(Isolation {
        cwd: work.clone(),
        root: work,
        branch: None,
        base_ref: None,
    })
}

/// A short account of what a run changed: `git diff --stat` against the base plus the files git
/// does not know yet, or for a copy the list of changed and new files.
pub fn diff_stat(p: &dyn Probe, run: &super::Run) -> String {
    let Some(root) = run.worktree.as_deref().map(Path::new) else {
        return String::new();
    };
    if let (Some(base), Some(_)) = (run.base_ref.as_deref(), run.branch.as_deref()) {
        let mut out = p
            .git(root, &["diff", "--stat", base])
            .unwrap_or_default()
            .trim()
            .to_string();
        if let Some(st) = p.git(root, &["status", "--porcelain", "--untracked-files=all"]) {
            let new: Vec<&str> = st.lines().filter(|l| l.starts_with("??")).collect();
            if !new.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("{} new file(s) not added to git:", new.len()));
                for l in new.iter().take(20) {
                    out.push_str(&format!("\n  {}", l[2..].trim()));
                }
            }
        }
        return out;
    }
    // a copy: compare with the original folder
    let orig = Path::new(&run.folder);
    let mut lines = vec![];
    fn walk(d: &Path, root: &Path, orig: &Path, lines: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                walk(&e.path(), root, orig, lines);
            } else if ft.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                match (std::fs::read(e.path()), std::fs::read(orig.join(&rel))) {
                    (Ok(a), Ok(b)) if a == b => {}
                    (Ok(_), Ok(_)) => lines.push(format!("changed  {}", rel.display())),
                    (Ok(_), Err(_)) => lines.push(format!("new      {}", rel.display())),
                    _ => {}
                }
            }
        }
    }
    walk(root, root, orig, &mut lines);
    lines.sort();
    lines.truncate(200);
    lines.join("\n")
}

/// Does the worktree have changes that no commit holds (tracked changes or new files)?
pub fn has_uncommitted(p: &dyn Probe, run: &super::Run) -> bool {
    match (&run.worktree, &run.branch) {
        (Some(wt), Some(_)) => p
            .git(
                Path::new(wt),
                &["status", "--porcelain", "--untracked-files=all"],
            )
            .is_some_and(|s| !s.trim().is_empty()),
        _ => false,
    }
}

/// Remove the run's worktree and branch (or its copy). Refuses while there are uncommitted changes
/// unless `lose_changes` says they may be lost. Only ever removes a folder inside `night_dir`, and
/// never passes `--force` to git: the folder is deleted and the worktree record pruned.
pub fn discard(
    p: &dyn Probe,
    night_dir: &Path,
    run: &super::Run,
    lose_changes: bool,
) -> Result<String> {
    anyhow::ensure!(
        !run.is_open() || run.worktree.is_none(),
        "run {} is still {}; stop it first",
        run.id,
        run.status
    );
    let Some(wt) = run.worktree.as_deref().map(PathBuf::from) else {
        bail!("this run has no folder to discard");
    };
    let inside = wt
        .canonicalize()
        .ok()
        .zip(night_dir.canonicalize().ok())
        .is_some_and(|(w, n)| w.starts_with(&n) && w != n);
    if !wt.exists() {
        // already gone: still drop the branch below
    } else if !inside {
        bail!(
            "{} is not inside eye's night folder; not removing it",
            wt.display()
        );
    } else if run.branch.is_some() && has_uncommitted(p, run) && !lose_changes {
        bail!(
            "the worktree has uncommitted changes that would be lost; to discard them too, run `eye night discard <id> --yes --lose-changes`"
        );
    }
    if wt.exists() {
        std::fs::remove_dir_all(&wt).with_context(|| format!("removing {}", wt.display()))?;
    }
    let mut said = format!("removed {}", wt.display());
    // a copy has its own repo, gone with its folder; the branch is only in the original for a worktree
    if let Some(b) = run.branch.as_ref().filter(|_| run.mode() == Mode::Worktree) {
        let repo = Path::new(&run.folder);
        let _ = p.git_act(repo, &["worktree", "prune"]);
        match p.git_act(repo, &["branch", "-D", b]) {
            Ok(_) => said.push_str(&format!(" and the branch {b}")),
            Err(e) => said.push_str(&format!("; the branch {b} was not deleted ({e})")),
        }
    }
    // the run's own folder (log, exit code) goes with it
    if let Some(dir) = wt
        .parent()
        .filter(|d| d.starts_with(night_dir) && *d != night_dir)
    {
        let _ = std::fs::remove_file(dir.join("exit"));
        let _ = std::fs::remove_dir(dir);
    }
    Ok(said)
}

/// Bring a repo copy's night branch into the original repo (`git fetch <copy> b:b`). Refuses a copy
/// that is not a repo copy, a copy that is gone, and a branch the original already has.
pub fn bring(p: &dyn Probe, run: &super::Run) -> Result<String> {
    anyhow::ensure!(
        run.is_repo_copy(),
        "only the copy of a git repo has a branch to bring into your repo"
    );
    let (Some(copy), Some(b)) = (run.worktree.as_deref(), run.branch.as_deref()) else {
        bail!("this run has no copy any more");
    };
    anyhow::ensure!(Path::new(copy).is_dir(), "the copy {copy} is gone");
    let repo = Path::new(&run.folder);
    let root = repo_root(p, repo).context("the original folder is not a git repo any more")?;
    anyhow::ensure!(
        branch_free(p, &root, b),
        "your repo already has a branch called {b}; rename or delete it first"
    );
    p.git_act(&root, &["fetch", "--no-tags", copy, &format!("{b}:{b}")])
        .map_err(|e| anyhow::anyhow!("git fetch failed: {e}"))?;
    Ok(format!("branch {b} is now in {}", root.display()))
}
