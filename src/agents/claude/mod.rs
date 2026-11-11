//! Claude Code: the first and default agent. It keeps its files under `~/.claude` (or the folder
//! in `[agents.claude] dir`, or `EYE_CLAUDE_DIR`): transcripts in `projects/`, live sessions in
//! `sessions/`, background sessions in `jobs/`, prompt history in `history.jsonl`, hooks in
//! `settings.json`. These files are Claude Code's own and can change with any release, so they are
//! read defensively and `eye doctor` checks them.

mod commands;
mod conversation;
mod headless;
mod hooks;
mod install;
mod live;
mod prompt;
mod signals;
mod transcript;

use super::*;
use crate::config::AgentCfg;
use crate::util::{find_in_path, mtime_ms};
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The agent id.
pub const ID: &str = "claude";
/// The short name used in sentences and `next` lines.
pub const SHORT: &str = "Claude";

/// The Claude Code adapter.
#[derive(Debug)]
pub struct Claude {
    dir: PathBuf,
    /// The program to run instead of `claude` (or `EYE_CLAUDE_BIN`).
    exe: Option<String>,
}

/// The factory the registry uses.
pub fn build(cfg: &AgentCfg) -> AgentRef {
    Arc::new(Claude::new(resolve_dir(cfg)))
}

/// Where Claude Code keeps its files: the CLI flag, then `EYE_CLAUDE_DIR`, then `[agents.claude]
/// dir`, then `~/.claude`.
fn resolve_dir(cfg: &AgentCfg) -> PathBuf {
    let from_env = std::env::var_os("EYE_CLAUDE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    cfg.dir_override
        .clone()
        .or(from_env)
        .or_else(|| cfg.dir.as_deref().map(crate::config::expand_tilde))
        .unwrap_or_else(|| home_dir().join(".claude"))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent"))
}

impl Claude {
    /// An adapter for Claude Code's files under `dir`.
    pub fn new(dir: PathBuf) -> Claude {
        Claude { dir, exe: None }
    }

    /// Run this program instead of `claude`.
    pub fn with_executable(mut self, exe: &str) -> Claude {
        self.exe = Some(exe.to_string());
        self
    }

    fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    fn skill_path(&self) -> PathBuf {
        self.dir.join("skills/eye/SKILL.md")
    }

    /// The program to run: set by `with_executable`, else `EYE_CLAUDE_BIN` (tests point it at a
    /// stub), else `claude` from PATH.
    fn executable_path(&self) -> String {
        self.exe
            .clone()
            .or_else(|| {
                std::env::var("EYE_CLAUDE_BIN")
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
}

/// Every `projects/<dir>/<session>.jsonl` under a Claude Code folder.
fn transcript_files(dir: &Path) -> Vec<Transcript> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(dir.join("projects")) else {
        return out;
    };
    for e in rd.flatten() {
        let project = e.path();
        if !project.is_dir() {
            continue;
        }
        let project_dir = e.file_name().to_string_lossy().into_owned();
        let Ok(rd2) = std::fs::read_dir(&project) else {
            continue;
        };
        for f in rd2.flatten() {
            let p = f.path();
            if p.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let (Some(stem), Ok(md)) = (p.file_stem().and_then(|s| s.to_str()), f.metadata())
            else {
                continue;
            };
            out.push(Transcript {
                project_dir: project_dir.clone(),
                session_id: stem.to_string(),
                path: p.clone(),
                stamp: Stamp::of_metadata(&md),
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A transcript file, buffered for reading line by line.
fn open_lines(path: &Path) -> Result<BufReader<std::fs::File>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Ok(BufReader::with_capacity(1 << 20, f))
}

impl Agent for Claude {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Claude Code"
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

    fn transcripts(&self) -> Vec<Transcript> {
        transcript_files(&self.dir)
    }

    /// The marker is the byte offset after the last complete line. A file that shrank or is older
    /// than at the last read was replaced, and is read again from the start.
    fn parse_transcript(&self, t: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        match append_only::resume_offset(opts.since, t.stamp) {
            Some(from) => transcript::parse_file(&t.path, from, opts),
            None => {
                let fresh = ParseOpts {
                    since: None,
                    carry: None,
                    last_msg_id: None,
                    ..*opts
                };
                let mut d = transcript::parse_file(&t.path, 0, &fresh)?;
                d.replaces = true;
                Ok(d)
            }
        }
    }

    fn owns_transcript(&self, path: &Path) -> bool {
        path.starts_with(self.dir.join("projects"))
    }

    fn own_dirs(&self) -> Vec<String> {
        let mut v = vec![self.dir.to_string_lossy().into_owned()];
        v.push(home_dir().join(".claude").to_string_lossy().into_owned());
        v
    }

    /// Sub-agents live in `<session>/subagents/`: a `<id>.meta.json` (description, finished) and a
    /// `<id>.jsonl` transcript each.
    fn subagents(&self, file: &Transcript) -> SubAgents {
        let dir = self
            .dir
            .join("projects")
            .join(&file.project_dir)
            .join(&file.session_id)
            .join("subagents");
        let mut out = SubAgents::default();
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return out;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                if let Some(stamp) = Stamp::of_file(&p) {
                    out.transcripts.push(Source { path: p, stamp });
                }
                continue;
            }
            let Some(id) = name.strip_suffix(".meta.json") else {
                continue;
            };
            let Ok(txt) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&txt) else {
                continue;
            };
            let agent_type = v.get("agentType").and_then(Value::as_str);
            let title = v
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or(agent_type)
                .unwrap_or(id);
            let finished = match v.get("finished") {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
                Some(Value::String(s)) => !s.is_empty(),
                _ => false,
            };
            out.children.push(ChildWork {
                id: id.to_string(),
                title: title.to_string(),
                detail: agent_type.map(String::from),
                finished,
                updated_at: e.metadata().map(|m| mtime_ms(&m)).unwrap_or(0),
            });
        }
        out
    }

    fn read_usage(
        &self,
        source: &Source,
        since: Option<LastRead>,
        last_id: Option<String>,
    ) -> Result<UsageRead> {
        let from = append_only::resume_offset(since, source.stamp);
        let last_id = last_id.filter(|_| from.is_some());
        let (tokens, last_id, end) =
            transcript::parse_usage_file(&source.path, from.unwrap_or(0), last_id)?;
        Ok(UsageRead {
            tokens,
            last_id,
            marker: end.to_string(),
            replaces: from.is_none(),
        })
    }

    fn history_source(&self) -> Option<Source> {
        let path = self.dir.join("history.jsonl");
        Some(Source {
            stamp: Stamp::of_file(&path)?,
            path,
        })
    }

    fn parse_history_line(&self, line: &Value) -> Option<HistoryPrompt> {
        Some(HistoryPrompt {
            session_id: line.get("sessionId")?.as_str()?.to_string(),
            text: line.get("display")?.as_str()?.to_string(),
            ts: line.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
            project: line
                .get("project")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
    }

    fn live_sessions(&self) -> Vec<LiveSession> {
        live::read_sessions(&self.dir.join("sessions"))
    }

    fn jobs(&self) -> Vec<Job> {
        live::read_jobs(&self.dir.join("jobs"))
    }

    fn queue_line(&self, event: &str, payload: &Value, ts: i64) -> Option<Value> {
        Some(hooks::queue_line(event, payload, ts))
    }

    fn wait_effect(&self, kind: &str, event: &Value, ts: i64) -> Option<Wait> {
        hooks::wait_effect(kind, event, ts)
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<crate::conversation::Message>> {
        Ok(conversation::conversation(&mut open_lines(path)?))
    }

    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        conversation::tail(&mut open_lines(path)?)
    }

    fn display_prompt(&self, raw: &str) -> Option<String> {
        prompt::display_prompt(raw)
    }
    fn clean_recap(&self, recap: &str) -> String {
        prompt::clean_recap(recap)
    }
    fn ask(&self, req: &HeadlessRequest) -> Result<HeadlessReply> {
        headless::ask(self, req)
    }
    fn default_summary_model(&self) -> &'static str {
        "haiku"
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

    fn background_job_id(&self, output: &str) -> Option<String> {
        commands::background_job_id(output)
    }

    fn daemon_env(&self) -> Vec<(String, String)> {
        if self.dir == home_dir().join(".claude") {
            vec![]
        } else {
            vec![(
                "EYE_CLAUDE_DIR".into(),
                self.dir.to_string_lossy().into_owned(),
            )]
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
