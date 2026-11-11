//! OpenAI's Codex CLI, the second agent. It keeps one rollout (a JSON-lines file) per session
//! under `$CODEX_HOME` (`~/.codex`): `sessions/YYYY/MM/DD/rollout-*.jsonl`, moved to
//! `archived_sessions/` by `codex archive`. Codex has no per-session status file, no job list and
//! no prompt history eye reads; its hooks, a writer lock per open session and the rollouts
//! themselves say what eye needs.

mod background;
mod commands;
mod conversation;
mod files;
mod goals;
mod headless;
mod hooks;
mod install;
mod live;
mod results;
mod rollout;
mod script;
mod work;

use super::*;
use crate::config::AgentCfg;
use crate::util::find_in_path;
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The agent id.
pub const ID: &str = "codex";
/// The short name used in sentences and `next` lines.
pub const SHORT: &str = "Codex";

/// The Codex CLI adapter.
#[derive(Debug)]
pub struct Codex {
    dir: PathBuf,
    /// The program to run instead of `codex` (or `EYE_CODEX_BIN`).
    exe: Option<String>,
    /// What the last look at the folder found (see [`Agent::transcripts`]): where each session is
    /// and which sub-agents a session started.
    index: Mutex<files::Index>,
}

/// The factory the registry uses.
pub fn build(cfg: &AgentCfg) -> AgentRef {
    Arc::new(Codex::new(resolve_dir(cfg)))
}

/// Where Codex keeps its files: `[agents.codex] dir`, then `$CODEX_HOME`, then `~/.codex`.
fn resolve_dir(cfg: &AgentCfg) -> PathBuf {
    cfg.dir_override
        .clone()
        .or_else(|| cfg.dir.as_deref().map(crate::config::expand_tilde))
        .or_else(|| {
            std::env::var_os("CODEX_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(default_dir)
}

fn default_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent"))
        .join(".codex")
}

impl Codex {
    /// An adapter for Codex's files under `dir`.
    pub fn new(dir: PathBuf) -> Codex {
        Codex {
            dir,
            exe: None,
            index: Mutex::default(),
        }
    }

    /// Run this program instead of `codex`.
    pub fn with_executable(mut self, exe: &str) -> Codex {
        self.exe = Some(exe.to_string());
        self
    }

    fn hooks_path(&self) -> PathBuf {
        self.dir.join("hooks.json")
    }

    fn skill_path(&self) -> PathBuf {
        self.dir.join("skills/eye/SKILL.md")
    }

    /// The program to run: set by `with_executable`, else `EYE_CODEX_BIN` (tests point it at a
    /// stub), else `codex` from PATH.
    fn executable_path(&self) -> String {
        self.exe
            .clone()
            .or_else(|| {
                std::env::var("EYE_CODEX_BIN")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| commands::PROGRAM.into())
    }

    fn on_path(&self) -> Option<PathBuf> {
        let exe = self.executable_path();
        if exe.contains('/') {
            Path::new(&exe).is_file().then(|| PathBuf::from(exe))
        } else {
            find_in_path(&exe)
        }
    }

    /// What the last look at the folder found. When there was none yet (a caller that only wants
    /// the live sessions, like `eye mark --current`) one is taken now.
    fn index(&self) -> std::sync::MutexGuard<'_, files::Index> {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.sessions.is_empty() {
            *index = files::scan(&self.dir).1;
        }
        index
    }
}

impl Agent for Codex {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Codex CLI"
    }
    fn short_name(&self) -> &'static str {
        SHORT
    }

    fn detect(&self) -> Detection {
        let dir = self.dir.is_dir();
        let exe = self.on_path();
        Detection {
            installed: dir || exe.is_some(),
            detail: match (dir, exe) {
                (true, _) => format!("found {}", self.dir.display()),
                (false, Some(p)) => format!("found {} on PATH", p.display()),
                (false, None) => format!(
                    "no {} and no `{}` on PATH",
                    self.dir.display(),
                    commands::PROGRAM
                ),
            },
        }
    }

    /// The sessions a person has: rollouts of `codex exec` runs, helper threads and sub-agents are
    /// left out (a sub-agent is a child stream of its parent). A session renamed after its last
    /// message shows the time of the rename as its change time, so it is read again.
    fn transcripts(&self) -> Vec<Transcript> {
        let (found, index) = files::scan(&self.dir);
        *self.index.lock().unwrap_or_else(|e| e.into_inner()) = index;
        found
    }

    /// The marker is the byte offset after the last complete line, and the carry says where the
    /// token total and the folder stood. A file that shrank or is older than at the last read was
    /// replaced; one whose folder changed or whose token total went down cannot be added to: both
    /// are read again from the start.
    fn parse_transcript(&self, t: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        let title = files::names(&self.dir)
            .get(&t.session_id)
            .map(|n| n.text.clone());
        let from = append_only::resume_offset(opts.since, t.stamp).filter(|_| opts.carry.is_some());
        if let Some(from) = from {
            let parsed = rollout::parse_file(&t.path, from, &t.session_id, opts, true)?;
            if !parsed.restart {
                let mut d = parsed.delta;
                d.custom_title = title;
                return Ok(d);
            }
        }
        let fresh = ParseOpts {
            since: None,
            carry: None,
            last_msg_id: None,
            ..*opts
        };
        let mut d = rollout::parse_file(&t.path, 0, &t.session_id, &fresh, false)?.delta;
        d.replaces = true;
        d.custom_title = title;
        Ok(d)
    }

    fn work_events(&self, path: &Path) -> Result<Vec<crate::worked::WorkEvent>> {
        work::read_events(path)
    }

    fn owns_transcript(&self, path: &Path) -> bool {
        [files::SESSIONS_DIR, files::ARCHIVED_DIR]
            .iter()
            .any(|d| path.starts_with(self.dir.join(d)))
    }

    fn own_dirs(&self) -> Vec<String> {
        let mut v = vec![self.dir.to_string_lossy().into_owned()];
        v.push(default_dir().to_string_lossy().into_owned());
        v
    }

    /// A sub-agent is a rollout of its own that names its parent. It is finished when Codex no
    /// longer holds its writer lock.
    fn subagents(&self, transcript: &Transcript) -> SubAgents {
        let index = self.index();
        let mut out = SubAgents::default();
        for c in index
            .children
            .get(&transcript.session_id)
            .into_iter()
            .flatten()
        {
            out.transcripts.push(Source {
                path: c.path.clone(),
                stamp: c.stamp,
            });
            out.children.push(ChildWork {
                id: c.id.clone(),
                title: c.title.clone(),
                detail: c.detail.clone(),
                finished: !live::writer_lock_held(&self.dir, &c.id),
                updated_at: c.stamp.mtime,
            });
        }
        out
    }

    /// The thread's token total is cumulative, so a sub-agent's usage is always read whole.
    fn read_usage(
        &self,
        source: &Source,
        _since: Option<LastRead>,
        last_id: Option<String>,
    ) -> Result<UsageRead> {
        Ok(UsageRead {
            tokens: rollout::last_total(&source.path)?.tokens(),
            last_id,
            marker: source.stamp.size.to_string(),
            replaces: true,
        })
    }

    fn live_sessions(&self) -> Vec<LiveSession> {
        live::sessions(&self.dir, &self.index())
    }

    /// Busy is an open turn, and a permission prompt writes no marker: a session stopped on one
    /// still reads as busy.
    fn live_status_shows_prompts(&self) -> bool {
        false
    }

    fn queue_line(&self, event: &str, payload: &Value, ts: i64) -> Option<Value> {
        Some(hooks::queue_line(event, payload, ts))
    }

    fn wait_effect(&self, kind: &str, event: &Value, ts: i64) -> Option<Wait> {
        hooks::wait_effect(kind, event, ts)
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<crate::conversation::Message>> {
        conversation::conversation(&mut open_lines(path)?)
    }

    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        conversation::tail(&mut open_lines(path)?)
    }

    fn default_summary_model(&self) -> &'static str {
        "gpt-6-luna"
    }

    fn ask(&self, req: &HeadlessRequest) -> Result<HeadlessReply> {
        headless::ask(self, req)
    }

    fn executable(&self) -> String {
        self.executable_path()
    }

    fn resume(&self, session_id: &str) -> Invocation {
        commands::resume(session_id)
    }

    fn fork(&self, session_id: &str) -> Option<Invocation> {
        Some(commands::fork(session_id))
    }

    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        commands::start(req)
    }

    fn daemon_env(&self) -> Vec<(String, String)> {
        if self.dir == default_dir() {
            vec![]
        } else {
            vec![("CODEX_HOME".into(), self.dir.to_string_lossy().into_owned())]
        }
    }

    fn executable_dir(&self) -> Option<PathBuf> {
        self.on_path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }

    fn preflight(&self, ctx: &InstallCtx) -> Result<()> {
        install::preflight(self, ctx)
    }

    fn install(&self, ctx: &InstallCtx) -> Result<Report> {
        install::install(self, ctx)
    }

    fn uninstall(&self, ctx: &InstallCtx) -> Result<Report> {
        install::uninstall(self, ctx)
    }

    fn doctor(&self, ctx: &InstallCtx) -> Vec<Check> {
        install::doctor(self, ctx)
    }
}

/// A rollout, buffered for reading line by line.
fn open_lines(path: &Path) -> Result<BufReader<std::fs::File>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(BufReader::with_capacity(1 << 20, f))
}
