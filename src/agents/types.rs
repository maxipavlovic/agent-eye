//! The common model between an agent adapter and the rest of eye: what a transcript parse
//! returns, what a live session or a background job looks like, and the commands an adapter
//! builds. Adapters fill these in; the scanner, board and picker only read them.

pub use crate::model::Stamp;
use crate::model::Tokens;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

/// Kept text per session for search (the most recent part wins).
pub const PROMPTS_CAP: usize = 80_000;
/// Kept reply text per session for search.
pub const REPLIES_CAP: usize = 120_000;

/// Is the agent there? Shown by `eye doctor`.
#[derive(Debug, Clone)]
pub struct Detection {
    pub installed: bool,
    /// One line for people: what was found, or where eye looked.
    pub detail: String,
}

/// What holds one session of an agent on disk: a file, a folder or a database. Only the adapter
/// looks inside `path`; the scanner keeps it as the session's key and checks `stamp` to see whether
/// the session changed.
#[derive(Debug, Clone)]
pub struct Transcript {
    /// Groups sessions of one project; also names the folder of the archive copy.
    pub project_dir: String,
    pub session_id: String,
    pub path: PathBuf,
    /// Changes whenever the session does. For a file: its size and modification time. For a folder
    /// or a database the adapter picks numbers that move with its content, for example the total
    /// size and the newest modification time of the files in it.
    pub stamp: Stamp,
}

/// A real output found in a transcript: a pull request, an artifact, a commit, a folder.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputHit {
    pub kind: &'static str,
    pub value: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub ts: i64,
}

/// A place an adapter reads besides a transcript, with the stamp that says whether it changed: a
/// sub-agent's transcript, the prompt history.
#[derive(Debug, Clone)]
pub struct Source {
    pub path: PathBuf,
    pub stamp: Stamp,
}

/// Where the last read of a transcript (or another source) stopped.
#[derive(Debug, Clone, Copy)]
pub struct LastRead<'a> {
    /// The marker the adapter returned then. The scanner stores it and never looks inside.
    pub marker: &'a str,
    /// The stamp the source had then, to compare with the stamp it has now.
    pub stamp: Stamp,
}

/// What one read of a transcript found: all of it, or what is new since the last read.
#[derive(Debug, Default)]
pub struct Delta {
    /// Where this read stopped, an opaque string the scanner stores and passes back as
    /// [`LastRead::marker`] at the next read. For a file read by byte offset it is the offset after
    /// the last complete line.
    pub marker: String,
    /// True when the adapter read the whole session and this delta stands for all of it, not only
    /// for what is new: its first read, a session that was rewritten, one it could not continue. The
    /// scanner then replaces what it holds instead of adding to it.
    pub replaces: bool,
    /// How much was read, for the scan statistics (bytes, rows, whatever is honest).
    pub bytes_read: u64,
    pub lines: u64,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub first_ts: Option<i64>,
    pub last_ts: Option<i64>,
    pub prompts_text: String,
    pub replies_text: String,
    pub prompt_count: u32,
    pub reply_count: u32,
    pub first_prompt: Option<String>,
    pub last_user_text: Option<String>,
    pub last_assistant_text: Option<String>,
    /// A title the user chose (`/rename`).
    pub custom_title: Option<String>,
    /// A title the agent wrote for the session.
    pub ai_title: Option<String>,
    /// A name the agent gave the session (a background session's task name).
    pub agent_title: Option<String>,
    /// The agent's own recap of the session, with its time.
    pub recap: Option<(i64, String)>,
    pub outputs: Vec<OutputHit>,
    /// Jira-shaped keys seen (key, ts); filtered against known projects when stored.
    pub jira: Vec<(String, i64)>,
    /// Project prefixes seen in `atlassian.net/browse/` URLs.
    pub jira_url_prefixes: BTreeSet<String>,
    /// The Jira site each project prefix was linked on (prefix -> host).
    pub jira_hosts: BTreeMap<String, String>,
    /// Project prefixes seen in git branch names.
    pub jira_branch_prefixes: BTreeSet<String>,
    /// Project prefixes typed in the user's own prompts.
    pub jira_prompt_prefixes: BTreeSet<String>,
    /// State the adapter wants back on the next read of the same session (e.g. tool calls whose
    /// result has not been read yet). The scanner stores it and does not look inside.
    pub carry: Option<Value>,
    pub folders: HashMap<String, u32>,
    /// Background-job tokens (short id or session id) seen in the transcript.
    pub child_hits: BTreeSet<String>,
    /// The session itself ran in the background.
    pub bg: bool,
    /// Token use of the lines read (`subagents_total` stays 0).
    pub usage: Tokens,
    /// Id of the last message counted in `usage`; carried to the next read so a message that
    /// spans several lines counts once.
    pub last_msg_id: Option<String>,
}

/// What the scanner hands to a transcript read besides the transcript itself.
#[derive(Clone, Copy)]
pub struct ParseOpts<'a> {
    /// Known job tokens; hits are reported in `Delta::child_hits`.
    pub job_tokens: &'a HashSet<String>,
    /// Folder outputs under these directories are dropped.
    pub ignore_dirs: &'a [String],
    /// Where the last read stopped. None: read the whole session (a first read, a full scan, a
    /// session that came back). Some: the adapter decides whether it can continue from the marker
    /// or must read everything again, and says so with `Delta::replaces`.
    pub since: Option<LastRead<'a>>,
    /// The `Delta::carry` of the last read of the same session. It goes with the marker: when the
    /// adapter reads everything again it leaves this out.
    pub carry: Option<&'a Value>,
    /// `Delta::last_msg_id` of the last read; to be left out in the same case.
    pub last_msg_id: Option<&'a str>,
}

/// Work a session started: a sub-agent. Shown as a child stream on the card.
#[derive(Debug, Clone)]
pub struct ChildWork {
    pub id: String,
    pub title: String,
    /// The kind of sub-agent, shown as the detail line.
    pub detail: Option<String>,
    pub finished: bool,
    pub updated_at: i64,
}

/// The sub-agents of one session.
#[derive(Debug, Default)]
pub struct SubAgents {
    pub children: Vec<ChildWork>,
    /// Their transcripts; only the token use is read from them (see `Agent::read_usage`).
    pub transcripts: Vec<Source>,
}

/// Token use of a sub-agent transcript: all of it, or what is new since the last read.
#[derive(Debug, Default)]
pub struct UsageRead {
    pub tokens: Tokens,
    pub last_id: Option<String>,
    /// Where this read stopped (see [`Delta::marker`]).
    pub marker: String,
    /// True when `tokens` count the whole transcript, not only what is new (see
    /// [`Delta::replaces`]).
    pub replaces: bool,
}

/// The prompts of an agent's history: all of them, or what is new since the last read.
#[derive(Debug, Default)]
pub struct HistoryRead {
    pub prompts: Vec<HistoryPrompt>,
    /// Where this read stopped (see [`Delta::marker`]).
    pub marker: String,
    /// True when `prompts` hold the whole history, not only what is new (see [`Delta::replaces`]).
    pub replaces: bool,
}

/// One prompt from an agent's prompt history (what survives after the transcripts are gone).
#[derive(Debug, Clone)]
pub struct HistoryPrompt {
    pub session_id: String,
    /// The folder the prompt was typed in.
    pub project: String,
    pub text: String,
    pub ts: i64,
}

/// A session that is open right now.
#[derive(Debug, Clone)]
pub struct LiveSession {
    pub pid: i64,
    pub session_id: String,
    pub cwd: Option<String>,
    /// `busy`, `idle` or `waiting`.
    pub status: Option<String>,
    pub updated_at: Option<i64>,
    /// How the session was started (an interactive terminal, a headless run).
    pub entrypoint: Option<String>,
    pub alive: bool,
}

/// A session the agent runs in the background.
#[derive(Debug, Clone)]
pub struct Job {
    pub short: String,
    pub state: String,
    pub detail: Option<String>,
    pub name: Option<String>,
    pub session_id: Option<String>,
    pub resume_session_id: Option<String>,
    pub cwd: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// How one hook event changes a session's wait: since when it waits on you, what it asks, and
/// whether a prompt stops the agent mid-task until you answer it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wait {
    pub since: Option<i64>,
    pub note: Option<String>,
    pub prompt: bool,
    /// Change the wait only while it is a prompt; any other wait stays as it is. For an event that
    /// shows a prompt was answered (a tool ran) and says nothing about a question in a reply.
    pub only_if_prompt: bool,
}

/// One item of the end of a conversation, for the summary excerpt.
#[derive(Debug, Clone, PartialEq)]
pub enum TailItem {
    /// A message: `role` is "user" or "assistant".
    Text { role: &'static str, text: String },
    /// A tool call or an error, as one line.
    Tool(String),
}

/// A one-shot structured question to the agent, run without a session (summaries).
pub struct HeadlessRequest<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub input: &'a str,
    /// JSON schema the answer must follow.
    pub schema: &'a Value,
    /// An empty folder to run in.
    pub work_dir: &'a std::path::Path,
    pub timeout: Duration,
}

/// The agent's answer to a [`HeadlessRequest`].
#[derive(Debug, Clone)]
pub struct HeadlessReply {
    /// The structured answer, shaped by the request's schema.
    pub output: Value,
    pub duration_ms: i64,
    /// Token use as the agent reports it (stored with the summary event).
    pub usage: Value,
    /// What the call cost, when the agent says (Codex does not).
    pub cost_usd: Option<f64>,
}

/// A program to run with its arguments. The folder is added when it is rendered or run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The name people know it by (`claude`); `Agent::executable` says what to really run.
    pub program: String,
    pub args: Vec<String>,
}

impl Invocation {
    /// An invocation of `program` with `args`.
    pub fn new(program: &str, args: &[&str]) -> Invocation {
        Invocation {
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// The line to paste into a terminal: `cd '<dir>' && <program> <args>`. Every argument that is
    /// not made of plain characters is single-quoted, so session data never reaches the shell as
    /// code. A folder that is not absolute follows `--`, so it cannot be read as an option of `cd`.
    pub fn shell_line(&self, dir: &str) -> String {
        let cd = if dir.starts_with('/') { "cd" } else { "cd --" };
        let mut line = format!("{cd} {} && {}", crate::util::shell_quote(dir), self.program);
        for a in &self.args {
            line.push(' ');
            line.push_str(&crate::util::shell_word(a));
        }
        line
    }
}

/// What to start for an idea.
pub struct StartRequest<'a> {
    pub dir: &'a str,
    pub title: &'a str,
    /// The first message of the new session; None opens the agent and waits for you.
    pub prompt: Option<&'a str>,
    pub background: bool,
    /// Reuse this session id (a start whose session never showed up); None picks a new one.
    pub session_id: Option<&'a str>,
}

/// The command that starts a session and the session id it will get, when the adapter can choose it.
#[derive(Debug, Clone)]
pub struct StartPlan {
    pub invocation: Invocation,
    pub session_id: Option<String>,
}

/// What `install` hands an adapter.
pub struct InstallCtx<'a> {
    /// The installed `agent-eye` binary the hooks call.
    pub eye_bin: &'a std::path::Path,
    pub dry_run: bool,
}

/// One line of `eye doctor`.
#[derive(Debug, Clone)]
pub struct Check {
    pub ok: bool,
    pub msg: String,
}

impl Check {
    /// A check that passed.
    pub fn ok(msg: impl Into<String>) -> Check {
        Check {
            ok: true,
            msg: msg.into(),
        }
    }
    /// A check that found a problem.
    pub fn warn(msg: impl Into<String>) -> Check {
        Check {
            ok: false,
            msg: msg.into(),
        }
    }
}
