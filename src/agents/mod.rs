//! The agent layer. Everything that is specific to one coding agent (where its sessions live, how a
//! transcript is read, its hooks, the commands that resume a session) sits behind the [`Agent`]
//! trait. The scanner, database, picker, board and daemon only talk to this trait, so another agent
//! can be added without touching them. Claude Code is the first and the default agent. See
//! `docs/agents.md`.

/// Reading a session that is one append-only text file.
pub mod append_only;
/// Claude Code, the first and default agent.
pub mod claude;
/// OpenAI's Codex CLI.
pub mod codex;
/// Adding eye's hooks to an agent's JSON hooks file and taking them out again.
pub mod hooks_file;
/// Finding Jira keys, pull requests and commits in transcript text, the same for every agent.
pub mod outputs;
/// Telling a reply that asks the user something from one that does not.
pub mod questions;
/// The `/eye` skill file of an agent.
pub mod skill_file;
mod types;

pub use types::*;

use crate::config::{AgentCfg, Config};
use crate::conversation::Message;
use crate::model::Session;
use crate::report::Report;
use crate::util::valid_session_id;
use anyhow::Result;
use serde_json::Value;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A shared handle to an agent adapter.
pub type AgentRef = Arc<dyn Agent>;

/// Everything eye needs to know about one coding agent. Required methods describe where its
/// sessions are and how to read them; the optional ones have a default that means "this agent does
/// not offer that" (no hooks, no background jobs, no forking, ...).
pub trait Agent: Send + Sync + std::fmt::Debug {
    // ---- identity ----

    /// Stable id: stored in the database and used in config and the API (`claude`).
    fn id(&self) -> &'static str;
    /// Full display name (`Claude Code`).
    fn name(&self) -> &'static str;
    /// Short name for sentences and the `next` owner (`Claude`).
    fn short_name(&self) -> &'static str;
    /// Is the agent on this machine?
    fn detect(&self) -> Detection;

    // ---- sessions on disk ----

    /// Every transcript of the agent, in a stable order.
    fn transcripts(&self) -> Vec<Transcript>;
    /// Read a transcript: all of it, or what is new since `opts.since`. The adapter decides which.
    /// It may go on from the marker of the last read or, when it cannot (the session was
    /// rewritten, the marker means nothing any more), read everything and set `Delta::replaces`.
    fn parse_transcript(&self, transcript: &Transcript, opts: &ParseOpts) -> Result<Delta>;
    /// True for a path that is one of the agent's transcripts (restoring an archived copy only
    /// writes there).
    fn owns_transcript(&self, path: &Path) -> bool;
    /// Folders that are the agent's own and never show up as a session's output.
    fn own_dirs(&self) -> Vec<String> {
        vec![]
    }
    /// Sub-agents of a session, as child streams and transcripts to count tokens from.
    fn subagents(&self, _transcript: &Transcript) -> SubAgents {
        SubAgents::default()
    }
    /// Token use of a sub-agent transcript: all of it, or what is new since `since`, the same way
    /// as `parse_transcript`. `last_id` is the last message id counted before; leave it out when
    /// reading everything again.
    fn read_usage(
        &self,
        _source: &Source,
        since: Option<LastRead>,
        last_id: Option<String>,
    ) -> Result<UsageRead> {
        Ok(UsageRead {
            last_id,
            marker: since.map(|s| s.marker.to_string()).unwrap_or_default(),
            ..UsageRead::default()
        })
    }
    /// Where the agent's prompt history is, with its stamp: what survives of sessions whose
    /// transcripts are deleted. None when the agent keeps none or it is not there.
    fn history_source(&self) -> Option<Source> {
        None
    }
    /// The prompts of the history, all of them or what is new since `since` (see
    /// `parse_transcript`). The default reads the source as an append-only file of JSON lines and
    /// hands each line to `parse_history_line`.
    fn read_history(&self, source: &Source, since: Option<LastRead>) -> Result<HistoryRead> {
        append_only::read_history(source, since, |line| self.parse_history_line(line))
    }
    /// One line of the history file, for the default `read_history`.
    fn parse_history_line(&self, _line: &Value) -> Option<HistoryPrompt> {
        None
    }

    // ---- archive ----

    /// The stamp of a transcript right now, None when it is gone. The archive copies a session again
    /// when it moved. The default is the stamp of a file; a folder or a database says its own.
    fn transcript_stamp(&self, path: &Path) -> Option<Stamp> {
        Stamp::of_file(path)
    }
    /// The name of a session's copy in the archive, inside `archive/[@<agent>/]<project dir>/`.
    fn archive_name(&self, session_id: &str) -> String {
        format!("{session_id}.jsonl.zst")
    }
    /// Keep a copy of the transcript at `path` by writing it to `copy`, which does not exist yet and
    /// may become a file or a folder. The default is one zstd file, which is right for a
    /// transcript that is one file. The result must restore to a transcript that reads the same.
    fn archive(&self, path: &Path, copy: &Path) -> Result<()> {
        crate::archive::compress_file(path, copy)
    }
    /// Put a copy made by `archive` back as a transcript at `dest`, which does not exist yet (the
    /// original place, or a temporary one when the copy is only read).
    fn unarchive(&self, copy: &Path, dest: &Path) -> Result<()> {
        crate::archive::decompress_file(copy, dest)
    }

    // ---- live state ----

    /// Sessions open right now (live status).
    fn live_sessions(&self) -> Vec<LiveSession> {
        vec![]
    }
    /// Does the live status show a permission prompt as `waiting`? An agent that says no keeps
    /// reporting `busy` while it is stopped on one, so a prompt a hook recorded wins over `busy`
    /// until a hook says it was answered (`Wait::only_if_prompt`). The default is yes: the status
    /// is trusted as it is.
    fn live_status_shows_prompts(&self) -> bool {
        true
    }
    /// Background sessions.
    fn jobs(&self) -> Vec<Job> {
        vec![]
    }

    // ---- hooks ----

    /// Turn a hook payload into a queue line. `event` is the name from the command line. Keep it
    /// short: the line is stored, so leave out prompts, file contents and tool output.
    fn queue_line(&self, _event: &str, _payload: &Value, _ts: i64) -> Option<Value> {
        None
    }
    /// How a queued event changes a session's wait (None: it does not).
    fn wait_effect(&self, _kind: &str, _event: &Value, _ts: i64) -> Option<Wait> {
        None
    }

    // ---- reading sessions ----

    /// The conversation of the transcript at `path`: real prompts and replies, oldest first. `path`
    /// exists: it is the transcript itself, or a copy made from the archive with `unarchive`. The
    /// adapter opens it its own way (a text file, a folder, a database).
    fn read_conversation(&self, path: &Path) -> Result<Vec<Message>>;
    /// The last messages and tool calls of the transcript at `path`, for the summary excerpt.
    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>>;
    /// A line of a session's stored prompt text as people should read it, or None when it is not
    /// something the user asked (a built-in command, an empty paste). The default only trims.
    fn display_prompt(&self, raw: &str) -> Option<String> {
        plain_prompt(raw)
    }
    /// A recap without what the agent adds to every one. The scanner stores recaps through this.
    fn clean_recap(&self, recap: &str) -> String {
        recap.trim().to_string()
    }
    /// Ask the agent one structured question without a session (summaries).
    fn ask(&self, _req: &HeadlessRequest) -> Result<HeadlessReply> {
        anyhow::bail!("{} cannot write summaries", self.name())
    }
    /// The model `ask` is given for summaries when `[summary] model` names none: a model that is
    /// cheap and fast, in the agent's own naming.
    fn default_summary_model(&self) -> &'static str {
        ""
    }

    // ---- commands ----

    /// The executable to run. `Invocation::program` is only the name shown to people.
    fn executable(&self) -> String;
    /// Resume a session.
    fn resume(&self, session_id: &str) -> Invocation;
    /// Resume a session as a new one that branches from it (None: the agent cannot fork).
    fn fork(&self, _session_id: &str) -> Option<Invocation> {
        None
    }
    /// Start a new session for an idea.
    fn start(&self, req: &StartRequest) -> Result<StartPlan>;
    /// The job id in what a background start printed.
    fn background_job_id(&self, _output: &str) -> Option<String> {
        None
    }

    // ---- install, uninstall, doctor ----

    /// Environment the daemon needs to find the agent's files (a non-default folder).
    fn daemon_env(&self) -> Vec<(String, String)> {
        vec![]
    }
    /// The folder of the agent's executable, so the daemon can start it.
    fn executable_dir(&self) -> Option<PathBuf> {
        None
    }
    /// Check that `install` can go ahead, before anything is written.
    fn preflight(&self, _ctx: &InstallCtx) -> Result<()> {
        Ok(())
    }
    /// Install hooks and skill. Returns the row for the install summary, and what the user has to
    /// do next, if anything.
    fn install(&self, _ctx: &InstallCtx) -> Result<Report> {
        Ok(Report::default())
    }
    /// Remove what `install` added. An agent with nothing of eye's to remove says nothing.
    fn uninstall(&self, _ctx: &InstallCtx) -> Result<Report> {
        Ok(Report::default())
    }
    /// Checks for `eye doctor`.
    fn doctor(&self, _ctx: &InstallCtx) -> Vec<Check> {
        vec![]
    }
}

/// A prompt line with nothing agent-specific to take out: trimmed, None when empty.
fn plain_prompt(raw: &str) -> Option<String> {
    let t = raw.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Builds an adapter from its `[agents.<id>]` settings.
pub type Factory = fn(&AgentCfg) -> AgentRef;

/// The id of the agent eye assumes when none is named: hooks without `--agent` and rows from
/// before eye knew several agents.
pub const LEGACY_ID: &str = "claude";

static EXTRA: Mutex<Vec<(String, Factory)>> = Mutex::new(Vec::new());

/// Add an agent to the registry (or replace one with the same id). It is used only when the config
/// enables it. This is how tests bring a second agent; a program that embeds eye can do the same.
pub fn register(id: &str, factory: Factory) {
    let mut extra = EXTRA.lock().unwrap_or_else(|e| e.into_inner());
    extra.retain(|(i, _)| i != id);
    extra.push((id.to_string(), factory));
}

fn factory(id: &str) -> Option<Factory> {
    let extra = EXTRA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, f)) = extra.iter().find(|(i, _)| i == id) {
        return Some(*f);
    }
    match id {
        claude::ID => Some(claude::build),
        codex::ID => Some(codex::build),
        _ => None,
    }
}

fn registered_ids() -> Vec<String> {
    let extra = EXTRA.lock().unwrap_or_else(|e| e.into_inner());
    let mut ids = vec![claude::ID.to_string(), codex::ID.to_string()];
    ids.extend(
        extra
            .iter()
            .map(|(i, _)| i.clone())
            .filter(|i| i != claude::ID && i != codex::ID),
    );
    ids
}

/// A list of agents in order; the first is the default.
#[derive(Clone, Default)]
pub struct Agents(Vec<AgentRef>);

impl Deref for Agents {
    type Target = [AgentRef];
    fn deref(&self) -> &[AgentRef] {
        &self.0
    }
}

impl Agents {
    /// A list of agents in the given order; the first is the default.
    pub fn from_vec(list: Vec<AgentRef>) -> Agents {
        Agents(list)
    }

    /// The agent with this id.
    pub fn get(&self, id: &str) -> Option<&AgentRef> {
        self.0.iter().find(|a| a.id() == id)
    }

    /// The first agent: used when a row or a request names none.
    pub fn default_agent(&self) -> Option<&AgentRef> {
        self.0.first()
    }

    /// The agent of a stored id; a missing or empty id means the default agent.
    pub fn resolve(&self, id: Option<&str>) -> Option<&AgentRef> {
        match id.filter(|i| !i.is_empty()) {
            Some(i) => self.get(i),
            None => self.default_agent(),
        }
    }

    /// The agent that owns a session.
    pub fn of_session(&self, s: &Session) -> Option<&AgentRef> {
        self.resolve(Some(s.agent.as_str()))
    }

    /// The agent that writes summaries: `[summary] agent`, else the default agent.
    pub fn summary_agent(&self, cfg: &Config) -> Option<&AgentRef> {
        match cfg.summary.agent.trim() {
            "" => self.default_agent(),
            id => self.get(id),
        }
    }

    /// The short name for an agent id, for text shown to people (an empty id is the default
    /// agent; the id itself when the agent is off).
    pub fn short_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.resolve(Some(id)).map_or(id, |a| a.short_name())
    }

    /// The names of the agents for a sentence: "Claude Code", "A or B", "A, B or C", or "your
    /// coding agent" when none is enabled.
    pub fn names_or(&self) -> String {
        let names: Vec<&str> = self.0.iter().map(|a| a.name()).collect();
        match names.as_slice() {
            [] => "your coding agent".into(),
            [only] => (*only).into(),
            [init @ .., last] => format!("{} or {last}", init.join(", ")),
        }
    }

    /// A line of a session's prompt text as people should read it (see [`Agent::display_prompt`]).
    /// A session of an agent that is off gets the plain reading.
    pub fn display_prompt(&self, s: &Session, raw: &str) -> Option<String> {
        match self.of_session(s) {
            Some(a) => a.display_prompt(raw),
            None => plain_prompt(raw),
        }
    }

    /// The agent and the folder to run a session's commands with, or why eye will not: the agent is
    /// off, the folder is unknown or has control characters in its name (a command line is typed
    /// into a terminal), or the id could be read as an option or a path.
    pub fn runnable<'a>(&'a self, s: &'a Session) -> Result<(&'a AgentRef, &'a str), String> {
        let agent = self
            .of_session(s)
            .ok_or_else(|| format!("the agent of this session ({}) is not enabled", s.agent))?;
        let cwd = match s.cwd.as_deref() {
            None | Some("") => return Err("this session has no known folder".into()),
            Some(c) if c.chars().any(char::is_control) => {
                return Err("the folder of this session has control characters in its name; eye will not put it in a command".into());
            }
            Some(c) => c,
        };
        if !valid_session_id(&s.id) {
            return Err("the id of this session is not one eye will put in a command".into());
        }
        Ok((agent, cwd))
    }

    /// The `resume` line of a session: `cd '<folder>' && <agent> --resume <id>`. None when
    /// [`Agents::runnable`] says no.
    pub fn resume_line(&self, s: &Session) -> Option<String> {
        let (agent, cwd) = self.runnable(s).ok()?;
        Some(agent.resume(&s.id).shell_line(cwd))
    }

    /// The same for a fork (None too when the agent cannot fork).
    pub fn fork_line(&self, s: &Session) -> Option<String> {
        let (agent, cwd) = self.runnable(s).ok()?;
        Some(agent.fork(&s.id)?.shell_line(cwd))
    }
}

/// The agents the config turns on, in order. `[agents] enabled` not set means Claude Code; ids
/// nobody registered are left out (`eye doctor` names them).
pub fn enabled(cfg: &Config) -> Agents {
    Agents(
        cfg.enabled_agent_ids()
            .iter()
            .filter_map(|id| factory(id).map(|f| f(&cfg.agent_cfg(id))))
            .collect(),
    )
}

/// Every registered agent, enabled or not: uninstall must clean up after all of them.
pub fn known(cfg: &Config) -> Agents {
    Agents(
        registered_ids()
            .iter()
            .filter_map(|id| factory(id).map(|f| f(&cfg.agent_cfg(id))))
            .collect(),
    )
}

/// Ids in `[agents] enabled` that no adapter is registered for.
pub fn unknown_ids(cfg: &Config) -> Vec<String> {
    cfg.enabled_agent_ids()
        .into_iter()
        .filter(|id| factory(id).is_none())
        .collect()
}

/// The adapter for a hook call (`hook <Event> [--agent <id>]`); no id means Claude Code. A hook
/// needs no config: reading the payload does not depend on where the agent keeps its files.
pub fn for_hook(id: Option<&str>) -> Option<AgentRef> {
    let id = id.filter(|i| !i.is_empty()).unwrap_or(LEGACY_ID);
    factory(id).map(|f| f(&AgentCfg::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> Session {
        Session {
            id: id.into(),
            cwd: Some("/work".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_folder_with_control_characters_gets_no_command_and_a_reason() {
        let agents = Agents::from_vec(vec![Arc::new(claude::Claude::new("/nonexistent".into()))]);
        for bad in [
            "/work/a\nrm -rf ~",
            "/work/a\u{1b}[2J",
            "/work/a\x7f",
            "/work/a\u{85}b",
            "/work/a\tb",
            "/work/a\0b",
        ] {
            let mut s = session("abcd-1234");
            s.cwd = Some(bad.into());
            assert!(agents.resume_line(&s).is_none(), "{bad:?}");
            assert!(agents.fork_line(&s).is_none(), "{bad:?}");
            let why = agents.runnable(&s).unwrap_err();
            assert!(why.contains("control characters"), "{why}");
        }
        // spaces, quotes and non-ASCII letters are fine: they are quoted
        let mut s = session("abcd-1234");
        s.cwd = Some("/work/it's ü".into());
        assert!(agents.resume_line(&s).is_some());
        // the other reasons, in words
        s.cwd = None;
        assert_eq!(
            agents.runnable(&s).unwrap_err(),
            "this session has no known folder"
        );
        let mut s = session("-x-id");
        assert!(agents
            .runnable(&s)
            .unwrap_err()
            .contains("id of this session"));
        s.agent = "gone".into();
        assert!(agents.runnable(&s).unwrap_err().contains("not enabled"));
    }

    #[test]
    fn agent_names_read_as_a_phrase() {
        assert_eq!(Agents::default().names_or(), "your coding agent");
        let one = Agents::from_vec(vec![Arc::new(claude::Claude::new("/x".into()))]);
        assert_eq!(one.names_or(), "Claude Code");
    }

    #[test]
    fn resume_and_fork_lines_refuse_ids_that_could_be_flags_or_paths() {
        let agents = Agents::from_vec(vec![Arc::new(claude::Claude::new("/nonexistent".into()))]);
        assert_eq!(
            agents.resume_line(&session("abcd-1234")).as_deref(),
            Some("cd '/work' && claude --resume abcd-1234")
        );
        assert!(agents.fork_line(&session("abcd-1234")).is_some());
        for bad in [
            "--dangerously-skip-permissions",
            "-x-id",
            "../up",
            "a b c d",
            "x;id",
            "ab",
        ] {
            assert!(agents.resume_line(&session(bad)).is_none(), "{bad}");
            assert!(agents.fork_line(&session(bad)).is_none(), "{bad}");
        }
        // no folder, no line
        let mut s = session("abcd-1234");
        s.cwd = None;
        assert!(agents.resume_line(&s).is_none());
        // an agent that is not enabled gives none either
        assert!(Agents::default()
            .resume_line(&session("abcd-1234"))
            .is_none());
    }
}
