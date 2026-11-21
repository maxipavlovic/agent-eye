//! Finding the loose ends: which session owns a process, a container, a worktree or a repo.
//! The attribution is pure (`Index`), so tests feed it fake process lists and fake hours.

use super::{gitscan, Docker, Kind, LooseEnd, Probe};
use crate::agents::Agents;
use crate::config::Config;
use crate::db::Db;
use crate::platform::procs::Proc;
use crate::util::{ago, home_rel};
use crate::worked;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A session as a scan needs it.
#[derive(Debug, Clone, Default)]
pub struct Sess {
    pub id: String,
    pub cwd: Option<String>,
    pub repo_root: Option<String>,
    pub last: i64,
    /// Open in an agent right now.
    pub live: bool,
}

/// What a scan reads from the database.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub sessions: Vec<Sess>,
    /// Events per (session, UTC hour) from `worked_hour`.
    pub events: HashMap<(String, i64), i64>,
    pub home: Option<PathBuf>,
}

/// Read the sessions active in the last `[loose] days` and their hours.
pub fn load(db: &Db, cfg: &Config, _agents: &Agents, now: i64) -> Result<Input> {
    let since = now - i64::from(cfg.loose.days) * 86_400_000;
    let mut st = db.conn.prepare(
        "SELECT id, cwd, repo_root, COALESCE(last_activity_at,0), live_status IS NOT NULL
         FROM session WHERE last_activity_at >= ?1 AND kind IS NOT 'history_only'",
    )?;
    let sessions: Vec<Sess> = st
        .query_map([since], |r| {
            Ok(Sess {
                id: r.get(0)?,
                cwd: r.get(1)?,
                repo_root: r.get(2)?,
                last: r.get(3)?,
                live: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut events = HashMap::new();
    let mut st = db.conn.prepare(
        "SELECT w.session_id, w.hour, w.events FROM worked_hour w
         JOIN session s ON s.id = w.session_id WHERE s.last_activity_at >= ?1",
    )?;
    for row in st.query_map([since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })? {
        let (id, hour, n) = row?;
        events.insert((id, hour), n);
    }
    Ok(Input {
        sessions,
        events,
        home: crate::platform::account_home(),
    })
}

/// `/private/var/x` and `/var/x` are the same folder on macOS: compare without the prefix.
pub(super) fn norm(p: &Path) -> PathBuf {
    match p.strip_prefix("/private") {
        Ok(rest) if rest.components().next().is_some() => Path::new("/").join(rest),
        _ => p.to_path_buf(),
    }
}

/// The folders of the sessions, ready to ask which contains a path.
pub(super) struct Index<'a> {
    pub input: &'a Input,
    by_id: HashMap<&'a str, usize>,
    /// Per session: its cwd and repo root, normalized; too broad ones left out.
    dirs: Vec<Vec<PathBuf>>,
    roots: Vec<PathBuf>,
    home: Option<PathBuf>,
}

impl<'a> Index<'a> {
    pub fn new(input: &'a Input, roots: &[PathBuf]) -> Index<'a> {
        let home = input.home.as_deref().map(norm);
        let broad = |p: &Path| p.components().count() < 3 || home.as_deref() == Some(p);
        let home_kept = home.clone();
        let dirs = input
            .sessions
            .iter()
            .map(|s| {
                let mut v: Vec<PathBuf> = [&s.cwd, &s.repo_root]
                    .into_iter()
                    .flatten()
                    .filter(|d| d.starts_with('/'))
                    .map(|d| norm(Path::new(d)))
                    .filter(|d| !broad(d))
                    .collect();
                v.dedup();
                v
            })
            .collect();
        Index {
            input,
            by_id: input
                .sessions
                .iter()
                .enumerate()
                .map(|(i, s)| (s.id.as_str(), i))
                .collect(),
            dirs,
            roots: roots.iter().map(|r| norm(r)).collect(),
            home: home_kept,
        }
    }

    /// Is this folder one a repo can live in: not the root, not the home folder?
    pub fn usable(&self, p: &Path) -> bool {
        let p = norm(p);
        p.components().count() >= 3 && self.home.as_deref() != Some(p.as_path())
    }

    /// The session whose id is a folder of `path`.
    pub fn by_session_id(&self, path: &Path) -> Option<usize> {
        let p = norm(path);
        // only below a scratchpad root, or anywhere: an id is long and unlike a folder name
        p.components()
            .filter_map(|c| c.as_os_str().to_str())
            .find_map(|c| self.by_id.get(c).copied())
    }

    pub fn in_scratch(&self, path: &Path) -> bool {
        let p = norm(path);
        self.roots.iter().any(|r| p.starts_with(r))
    }

    /// Which session owns something that lives in `path` and began at `start_ms`: the session
    /// whose id is in the path, else the one whose folder contains it and that worked in that
    /// hour (most events wins; the newer session on a tie).
    pub fn attribute(&self, path: &Path, start_ms: i64) -> Option<usize> {
        if let Some(i) = self.by_session_id(path) {
            return Some(i);
        }
        let p = norm(path);
        let hour = worked::hour_of(start_ms);
        self.input
            .sessions
            .iter()
            .enumerate()
            .filter(|(i, _)| self.dirs[*i].iter().any(|d| p.starts_with(d)))
            .filter_map(|(i, s)| {
                let n = *self.input.events.get(&(s.id.clone(), hour))?;
                (n > 0).then_some((n, s.last, i))
            })
            .max()
            .map(|(_, _, i)| i)
    }
}

/// Programs that are never a leftover: the agents, eye, shells, editors, terminals, browsers.
fn ignored_program(command: &str, has_port: bool) -> bool {
    let first = command.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    let base = base.trim_start_matches('-').to_lowercase();
    const NEVER: &[&str] = &[
        "claude", "codex", "eye", "agent-eye", "sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh",
        "csh", "nu", "login", "tmux", "screen", "sshd", "ssh", "ssh-agent", "vim", "nvim", "vi",
        "emacs", "nano", "less", "more", "man", "code", "cursor", "zed", "subl", "idea", "ps",
        "lsof", "docker", "docker-proxy", "dockerd", "containerd", "colima", "limactl",
        "firefox", "chrome", "safari", "brave", "iterm2", "terminal", "ghostty", "alacritty",
        "kitty", "wezterm", "git", "caffeinate", "pbcopy",
    ];
    if NEVER.contains(&base.as_str()) || base.starts_with("com.docker") {
        return true;
    }
    let lower = command.to_lowercase();
    if lower.contains(".app/contents/") {
        return true;
    }
    // helpers of an editor or an agent talk over a pipe: only a port makes them a leftover
    !has_port
        && ["mcp", "language-server", "language_server", "rust-analyzer", "gopls", "pyright",
            "tsserver", "typescript-language", "eslint_d", "copilot", "lsp", "ripgrep", "fsevents"]
            .iter()
            .any(|h| lower.contains(h))
}

/// `node vite (pid 4)`: the program, and for an interpreter its script.
fn label(command: &str) -> String {
    let mut it = command.split_whitespace();
    let first = it.next().unwrap_or("");
    let prog = first.rsplit('/').next().unwrap_or(first);
    let interp = [
        "node", "python", "python3", "ruby", "java", "bun", "deno", "php", "perl", "npm", "npx",
        "pnpm", "yarn", "uv", "uvx", "tsx", "ts-node",
    ];
    let mut out = prog.to_string();
    if interp.contains(&prog) || prog.starts_with("python") {
        if let Some(arg) = it.find(|a| !a.starts_with('-')) {
            out.push(' ');
            out.push_str(arg.rsplit('/').next().unwrap_or(arg));
        }
    }
    crate::util::head_chars(&out, 40)
}

fn ports_text(ports: &[u16]) -> String {
    match ports {
        [] => String::new(),
        [p] => format!("port {p}"),
        many => format!(
            "ports {}",
            many.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn process_item(p: &Proc, sid: &str, now: i64) -> LooseEnd {
    let name = label(&p.command);
    let ports = ports_text(&p.ports);
    let what = if ports.is_empty() {
        format!("{name} (pid {})", p.pid)
    } else {
        format!("{name} (pid {}, {ports})", p.pid)
    };
    let started = ago(now, p.start * 1000);
    let line = if ports.is_empty() {
        format!("{name} (pid {}) is still running, started {started} ago", p.pid)
    } else {
        format!("{name} (pid {}) is still serving {ports}, started {started} ago", p.pid)
    };
    let cwd = p.cwd.as_deref().map(|c| home_rel(&c.to_string_lossy()));
    let mut detail = what;
    if let Some(c) = cwd {
        detail.push_str(&format!("\nin {c}"));
    }
    detail.push_str(&format!("\n{}", crate::util::head_chars(&p.command, 200)));
    LooseEnd {
        key: format!("proc:{}:{}", p.pid, p.start),
        session_id: sid.into(),
        kind: Kind::Process,
        line,
        detail,
        since_ms: p.start * 1000,
    }
}

fn container_item(c: &Docker, sid: &str, now: i64) -> LooseEnd {
    let short: String = c.id.chars().take(12).collect();
    let what = format!("container {} ({short})", c.name);
    LooseEnd {
        key: format!("box:{}:{}", c.id, c.created),
        session_id: sid.into(),
        kind: Kind::Container,
        line: format!(
            "container {} ({}) is still running, created {} ago",
            c.name,
            c.image,
            ago(now, c.created * 1000)
        ),
        detail: format!("{what}\nimage {}", c.image),
        since_ms: c.created * 1000,
    }
}

/// Every loose end, found through `probe`. Items of one session stay together, newest first.
pub fn find(input: &Input, probe: &dyn Probe, now: i64) -> Vec<LooseEnd> {
    let roots = probe.scratch_roots();
    let idx = Index::new(input, &roots);
    let mut out: Vec<LooseEnd> = vec![];
    if !input.sessions.is_empty() {
        // processes
        let want = |cmd: &str| !ignored_program(cmd, true);
        for p in probe.processes(&want) {
            if ignored_program(&p.command, !p.ports.is_empty()) {
                continue;
            }
            let Some(cwd) = p.cwd.as_deref() else {
                continue;
            };
            if let Some(i) = idx.attribute(cwd, p.start * 1000) {
                out.push(process_item(&p, &input.sessions[i].id, now));
            }
        }
        // containers
        for c in probe.docker() {
            let hit = c
                .workdir
                .iter()
                .chain(c.mounts.iter())
                .find_map(|d| idx.attribute(Path::new(d), c.created * 1000));
            if let Some(i) = hit {
                out.push(container_item(&c, &input.sessions[i].id, now));
            }
        }
        out.extend(gitscan::find(&idx, probe, now));
    }
    out.sort_by(|a, b| {
        (&a.session_id, a.kind, std::cmp::Reverse(a.since_ms), &a.key)
            .cmp(&(&b.session_id, b.kind, std::cmp::Reverse(b.since_ms), &b.key))
    });
    out
}
