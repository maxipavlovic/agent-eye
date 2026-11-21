//! Google's Antigravity CLI (`agy`). A conversation is a SQLite database
//! (`conversations/<id>.db`, protobuf blobs) with a folder `brain/<id>/` beside it. eye reads the
//! plain JSON log agy writes in that folder (`.system_generated/logs/transcript_full.jsonl`, see
//! `log.rs`), the model of each call from the database, titles and folders from
//! `conversation_summaries.db` and prompts from `history.jsonl`. The session is the folder
//! `brain/<id>/`. Live status comes from `presence/<id>.lock`, the summaries row and the last step
//! of the database (`live.rs`); hooks are a plugin of eye's in `~/.gemini/config/plugins`
//! (`install.rs`); sub-agent conversations are child streams of their parent.

mod archive;
mod cli_log;
mod commands;
mod files;
mod headless;
mod hooks;
mod install;
mod live;
mod log;
mod wire;

use super::*;
use crate::config::AgentCfg;
use crate::util::find_in_path;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The agent id.
pub const ID: &str = "agy";
/// The short name used in sentences and `next` lines.
pub const SHORT: &str = "Antigravity";

/// The Antigravity CLI adapter.
#[derive(Debug)]
pub struct Agy {
    dir: PathBuf,
    exe: Option<String>,
    /// Titles and folders found by the last look at the folder.
    sums: Mutex<HashMap<String, files::Summary>>,
}

/// The factory the registry uses.
pub fn build(cfg: &AgentCfg) -> AgentRef {
    Arc::new(Agy::new(resolve_dir(cfg)))
}

/// `[agents.agy] dir`, else `~/.gemini/antigravity-cli`.
fn resolve_dir(cfg: &AgentCfg) -> PathBuf {
    cfg.dir_override
        .clone()
        .or_else(|| cfg.dir.as_deref().map(crate::config::expand_tilde))
        .unwrap_or_else(default_dir)
}

fn default_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/nonexistent"))
        .join(".gemini/antigravity-cli")
}

impl Agy {
    /// An adapter for agy's files under `dir`.
    pub fn new(dir: PathBuf) -> Agy {
        Agy {
            dir,
            exe: None,
            sums: Mutex::default(),
        }
    }

    /// Run this program instead of `agy`.
    pub fn with_executable(mut self, exe: &str) -> Agy {
        self.exe = Some(exe.to_string());
        self
    }

    /// The program to run: `with_executable`, else `EYE_AGY_BIN`, else `agy` from PATH, else the
    /// installer's `~/.local/bin/agy` (a daemon's PATH may lack it).
    fn executable_path(&self) -> String {
        if let Some(e) = self
            .exe
            .clone()
            .or_else(|| std::env::var("EYE_AGY_BIN").ok().filter(|s| !s.is_empty()))
        {
            return e;
        }
        if find_in_path(commands::PROGRAM).is_none() {
            if let Some(h) = std::env::var_os("HOME") {
                let p = Path::new(&h).join(".local/bin/agy");
                if p.is_file() {
                    return p.to_string_lossy().into_owned();
                }
            }
        }
        commands::PROGRAM.into()
    }

    fn on_path(&self) -> Option<PathBuf> {
        let exe = self.executable_path();
        if exe.contains('/') {
            Path::new(&exe).is_file().then(|| PathBuf::from(exe))
        } else {
            find_in_path(&exe)
        }
    }

    /// agy's global customization folder, `~/.gemini/config`, next to its own.
    fn config_dir(&self) -> PathBuf {
        self.dir
            .parent()
            .map_or_else(|| self.dir.join("config"), |p| p.join("config"))
    }

    /// eye's plugin (its hooks), in the global plugins folder.
    fn plugin_dir(&self) -> PathBuf {
        self.config_dir().join("plugins").join(install::PLUGIN)
    }

    fn skill_path(&self) -> PathBuf {
        self.config_dir().join("skills/eye/SKILL.md")
    }

    /// How conversation `id` was started, as agy says it: the `source` of its summaries row, else
    /// `print-mode` when agy's own log shows a `-p` run.
    fn origin_of(&self, id: &str) -> Option<String> {
        let source = self.summary(id).source;
        if !source.is_empty() {
            return Some(source);
        }
        cli_log::headless(&self.dir, id).then(|| "print-mode".to_string())
    }

    fn summary(&self, id: &str) -> files::Summary {
        let cached = self
            .sums
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        cached.unwrap_or_else(|| files::summaries(&self.dir).remove(id).unwrap_or_default())
    }
}

/// The models of the model calls of the conversation in `folder`, in order.
fn models_of(folder: &Path) -> Vec<String> {
    let Some(db) = files::db_of(folder) else {
        return vec![];
    };
    let Some(c) = files::open_ro(&db) else {
        return vec![];
    };
    let Ok(mut st) = c.prepare("SELECT data FROM gen_metadata ORDER BY idx") else {
        return vec![];
    };
    let rows = st.query_map([], |r| r.get::<_, Vec<u8>>(0));
    rows.into_iter()
        .flatten()
        .flatten()
        .map(|b| wire::model_name(&b).unwrap_or_default())
        .collect()
}

impl Agent for Agy {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Antigravity CLI"
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
        let (found, sums) = files::scan(&self.dir);
        *self.sums.lock().unwrap_or_else(|e| e.into_inner()) = sums;
        found
    }

    /// The marker is the byte offset after the last complete line of the log and the carry counts
    /// the model calls read (to pick each call's model). A log that shrank or is older than at the
    /// last read was replaced and is read again from the start.
    fn parse_transcript(&self, t: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        let log = files::log_of(&t.path);
        let models = || models_of(&t.path);
        let resumed = append_only::resume_offset(opts.since, t.stamp).zip(
            opts.carry
                .and_then(|c| c.get("calls"))
                .and_then(Value::as_u64),
        );
        let (from, before) = resumed.unwrap_or((0, 0));
        let p = log::parse(&log, from, before as usize, &models)?;
        let mut d = p.delta;
        d.replaces = resumed.is_none();
        d.session_id = Some(t.session_id.clone());
        let s = self.summary(&t.session_id);
        d.cwd = s.folder;
        d.ai_title = s.title;
        d.carry = Some(serde_json::json!({ "calls": p.model_calls }));
        if resumed.is_none() {
            // asked on a whole read only: it looks through agy's process logs
            d.origin_raw = self.origin_of(&t.session_id);
        }
        Ok(d)
    }

    fn work_events(&self, path: &Path) -> Result<Vec<WorkEvent>> {
        log::work_events(&files::log_of(path))
    }

    fn usage_msgs(&self, path: &Path) -> Result<Vec<UsageMsg>> {
        log::usage_msgs(&files::log_of(path), &|| models_of(path))
    }

    fn origin_raw(&self, path: &Path) -> Option<String> {
        self.origin_of(path.file_name()?.to_str()?)
    }

    /// Sub-agent conversations (agy's `invoke_subagent`) below the session, as child streams. The
    /// title is the role the parent gave (`brain/<parent>/.system_generated/subagents/<id>.json`),
    /// the detail the kind (`self`). A child is finished when its turn is not running.
    fn subagents(&self, t: &Transcript) -> SubAgents {
        let sums = files::summaries(&self.dir);
        let mut out = SubAgents::default();
        for id in files::descendants(&sums, &t.session_id) {
            let s = &sums[&id];
            let folder = self.dir.join(files::BRAIN).join(&id);
            let Some(stamp) = files::stamp(&folder, s.modified) else {
                continue;
            };
            let manifest = s
                .parent
                .as_deref()
                .and_then(|p| files::subagent_manifest(&self.dir, p, &id));
            let (role, kind) = manifest.unwrap_or_default();
            out.transcripts.push(Source {
                path: folder,
                stamp,
            });
            out.children.push(ChildWork {
                id,
                title: Some(role)
                    .filter(|r| !r.is_empty())
                    .or_else(|| s.title.clone())
                    .unwrap_or_else(|| "sub-agent".into()),
                detail: Some(kind)
                    .filter(|k| !k.is_empty())
                    .or_else(|| Some(s.agent.clone()).filter(|a| !a.is_empty())),
                finished: !s.running,
                updated_at: stamp.mtime,
            });
        }
        out
    }

    /// A sub-agent's conversation is a log like any other: read whole, the tokens summed.
    fn read_usage(
        &self,
        source: &Source,
        _since: Option<LastRead>,
        last_id: Option<String>,
    ) -> Result<UsageRead> {
        let msgs = log::usage_msgs(&files::log_of(&source.path), &|| models_of(&source.path))?;
        let mut tokens = crate::model::Tokens::default();
        for m in &msgs {
            tokens.input += m.input;
            tokens.output += m.output;
            tokens.cache_read += m.cache_read;
        }
        Ok(UsageRead {
            tokens,
            msgs,
            last_id,
            marker: source.stamp.size.to_string(),
            replaces: true,
        })
    }

    fn live_sessions(&self) -> Vec<LiveSession> {
        live::sessions(&self.dir, &files::summaries_with_folders(&self.dir))
    }

    /// The status is read from the database, where agy writes the approval it waits for: a
    /// `waiting` session is trusted as it is.
    fn live_status_shows_prompts(&self) -> bool {
        true
    }

    fn queue_line(&self, event: &str, payload: &Value, ts: i64) -> Option<Value> {
        Some(hooks::queue_line(event, payload, ts))
    }

    fn wait_effect(&self, kind: &str, event: &Value, ts: i64) -> Option<Wait> {
        hooks::wait_effect(kind, event, ts)
    }

    fn owns_transcript(&self, path: &Path) -> bool {
        path.starts_with(self.dir.join(files::BRAIN))
    }

    /// The conversation's folder under `brain/`, its database with the `-shm` and `-wal` files, and
    /// its row in `conversation_summaries.db`.
    fn delete_transcript(&self, path: &Path, session_id: &str) -> Result<Option<Vec<String>>> {
        files::delete(&self.dir, path, session_id).map(Some)
    }

    fn own_dirs(&self) -> Vec<String> {
        vec![self.dir.to_string_lossy().into_owned()]
    }

    fn history_source(&self) -> Option<Source> {
        let path = self.dir.join("history.jsonl");
        Some(Source {
            stamp: Stamp::of_file(&path)?,
            path,
        })
    }

    /// A prompt typed in a conversation. Slash commands, `exit` and lines typed before the
    /// conversation existed (no `conversationId`) are not prompts of a session.
    fn parse_history_line(&self, line: &Value) -> Option<HistoryPrompt> {
        if line.get("type").and_then(Value::as_str).is_some() {
            return None;
        }
        let text = line.get("display")?.as_str()?.trim();
        if text.is_empty() || text == "exit" {
            return None;
        }
        Some(HistoryPrompt {
            session_id: line.get("conversationId")?.as_str()?.to_string(),
            project: line
                .get("workspace")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            text: text.to_string(),
            ts: line.get("timestamp").and_then(Value::as_i64).unwrap_or(0),
        })
    }

    fn transcript_stamp(&self, path: &Path) -> Option<Stamp> {
        files::stamp(path, 0)
    }

    fn archive_name(&self, session_id: &str) -> String {
        session_id.to_string()
    }

    fn archive(&self, path: &Path, copy: &Path) -> Result<()> {
        archive::archive(path, copy)
    }

    /// Restoring into agy's own `brain/` also puts the database and the summaries row back, so
    /// that `agy --conversation <id>` opens the conversation after agy deleted it.
    fn unarchive(&self, copy: &Path, dest: &Path) -> Result<()> {
        archive::unarchive(copy, dest)?;
        // the copy is named by the conversation id; `dest` is a sibling while it is restored
        let id = copy.file_name().and_then(|n| n.to_str());
        if let Some(id) = id.filter(|i| valid_session_id(i)) {
            if self.owns_transcript(dest) {
                archive::reinstall(&self.dir, id, dest)?;
            }
        }
        Ok(())
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<crate::conversation::Message>> {
        log::conversation(&files::log_of(path))
    }

    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        log::tail(&files::log_of(path))
    }

    fn default_summary_model(&self) -> &'static str {
        "gemini-3.8-flash-low"
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

    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        commands::start(req)
    }

    fn preflight(&self, ctx: &InstallCtx) -> Result<()> {
        install::preflight(self, ctx)
    }

    fn install(&self, ctx: &InstallCtx) -> Result<Report> {
        install::install(self, ctx)
    }

    fn refresh_skill(&self) -> Result<Option<skill_file::State>> {
        skill_file::refresh(&self.skill_path(), install::SKILL_MD)
    }

    fn uninstall(&self, ctx: &InstallCtx) -> Result<Report> {
        install::uninstall(self, ctx)
    }

    fn doctor(&self, ctx: &InstallCtx) -> Vec<Check> {
        install::doctor(self, ctx)
    }

    fn executable_dir(&self) -> Option<PathBuf> {
        self.on_path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }
}
