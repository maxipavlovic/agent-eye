//! Codex rollouts: one JSON object per line, `{"timestamp", "type", "payload"}`, in two dialects.
//! A legacy rollout writes what the user and the agent said as `event_msg` lines (`user_message`,
//! `agent_message`); a paginated one writes them as `event_msg` `item_completed` lines holding a
//! typed item. Both carry the model's own history as `response_item` lines, which holds injected
//! context (`AGENTS.md`, the environment) next to the real tool calls, so prompts are only taken
//! from the first kind of line.
//!
//! A paginated rollout (every line has an `ordinal`) records each tool call twice: as the model's
//! raw `response_item` and as an `item_completed` item (`CommandExecution`, `FileChange`, ...) that
//! holds the command, its output and exit code, or the files changed. Tools run through Codex's
//! code mode there, so the raw call is a JavaScript program. The items are what is read, and the
//! raw calls of such a file are skipped; a legacy rollout has only the raw calls. [`decode`] reads
//! a line in either dialect; [`Reader`] turns the lines of a file into a [`Delta`].

use super::background::Tracker;
use super::script::{self, Call};
use super::work::work_event;
use crate::agents::append_only::for_each_line;
use crate::agents::outputs::{
    commit_message, commit_sha, dir_ignored, is_git_commit, push_output, scan_jira,
    scan_jira_branch, scan_jira_urls, scan_pr, Seen,
};
use crate::agents::{Delta, ParseOpts, PROMPTS_CAP, REPLIES_CAP};
use crate::model::Tokens;
use crate::util::{head_chars, iso_to_ms, tail_chars, trim_front_bytes};
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

const MSG_CAP: usize = 12_000;
/// The first line of an `apply_patch` text.
pub const PATCH_START: &str = "*** Begin Patch";
/// Tool calls whose result is still to come that are remembered between two reads.
const PENDING_CAP: usize = 64;
/// The words before an IDE-made prompt's own text.
const IDE_MARKER: &str = "## My request for Codex:";
/// What Codex injects as a user message: never something the user typed.
const INJECTED: &[&str] = &[
    "# AGENTS.md instructions for",
    "<environment_context>",
    "<user_shell_command>",
    "<turn_aborted>",
    "<subagent_notification>",
    "<skill>",
    "<model_switch>",
    "<current_time_reminder>",
    "<codex_internal_context",
];
/// A line this long whose type cannot be told from its start is not parsed.
const NO_TYPE_PARSE_MAX: usize = 64 << 20;
/// Lines of these types are read; the rest (`world_state`, `retained_context`, ...) is skipped
/// before it is parsed.
const TYPES: &[&str] = &[
    "session_meta",
    "turn_context",
    "event_msg",
    "response_item",
    "compacted",
    "token_usage_record",
];

static RE_EXIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Process exited with code (-?\d+)").expect("static regex"));

/// The tokens of a thread as Codex counts them. The numbers in a rollout are cumulative for the
/// thread: the newest one is the total, never a sum of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens, those served from the cache included.
    pub input: i64,
    pub cached: i64,
    pub cache_write: i64,
    /// Output tokens, reasoning included.
    pub output: i64,
}

impl Usage {
    fn of(v: &Value) -> Usage {
        let n = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0).max(0);
        Usage {
            input: n("input_tokens"),
            cached: n("cached_input_tokens"),
            cache_write: n("cache_write_input_tokens"),
            output: n("output_tokens"),
        }
    }

    /// In eye's terms: input without what came from the cache, which is counted by itself.
    pub fn tokens(&self) -> Tokens {
        Tokens {
            input: (self.input - self.cached).max(0),
            output: self.output,
            cache_write: self.cache_write,
            cache_read: self.cached,
            subagents_total: 0,
        }
    }

    /// What was added since `before`; None when a count went down (the thread was rewound).
    fn since(&self, before: &Usage) -> Option<Usage> {
        let up = self.input >= before.input
            && self.cached >= before.cached
            && self.cache_write >= before.cache_write
            && self.output >= before.output;
        up.then(|| Usage {
            input: self.input - before.input,
            cached: self.cached - before.cached,
            cache_write: self.cache_write - before.cache_write,
            output: self.output - before.output,
        })
    }
}

/// A tool call of the model that matters to eye.
#[derive(Debug, Clone, PartialEq)]
pub enum Tool {
    /// A shell command.
    Shell(String),
    /// An `apply_patch` edit: the patch text.
    Patch(String),
    /// Files a paginated rollout says were changed, by path.
    Files(Vec<String>),
    /// A code mode program (`exec`): the calls read from its source, none when it cannot be read.
    Script(Vec<Tool>),
    /// A tool that is shown by name only (an MCP tool, a web search).
    Named(String),
    /// Anything else (a plan update).
    Other,
}

/// One line of a rollout, reduced to what eye uses.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The thread's own start: its folder and git branch.
    Meta {
        id: Option<String>,
        cwd: Option<String>,
        branch: Option<String>,
    },
    /// A turn begins in this folder.
    Turn {
        cwd: Option<String>,
    },
    /// What the user typed.
    Prompt(String),
    /// What the agent said.
    Reply(String),
    Call {
        id: String,
        tool: Tool,
    },
    Output {
        call_id: String,
        text: String,
    },
    /// A tool call and its result as one record (paginated rollouts).
    Done {
        tool: Tool,
        output: String,
        exit: Option<i64>,
    },
    /// The thread's token use so far.
    Total(Usage),
    /// The context was compacted.
    Compacted,
    Other,
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// The text of a content list: `[{"type": "text" | "input_text" | "output_text", "text"}]`.
fn joined(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// A user message as a prompt: what an IDE put before the user's own text is taken off, and what
/// Codex injected (`AGENTS.md`, the environment, a notification) is not a prompt at all.
pub fn prompt_text(raw: &str) -> Option<String> {
    let t = raw
        .split_once(IDE_MARKER)
        .map_or(raw, |(_, rest)| rest)
        .trim();
    (!t.is_empty() && !INJECTED.iter().any(|p| t.starts_with(p))).then(|| t.to_string())
}

/// The exit code a command's output reports: the `Process exited with code N` header of the
/// unified exec tool, or `metadata.exit_code` of an older shell tool's JSON.
pub fn exit_code(text: &str) -> Option<i64> {
    let head = text
        .split_once("Output:")
        .map_or_else(|| head_chars(text, 500), |(head, _)| head_chars(head, 500));
    if let Some(c) = RE_EXIT.captures(&head) {
        return c[1].parse().ok();
    }
    serde_json::from_str::<Value>(text.trim())
        .ok()?
        .pointer("/metadata/exit_code")?
        .as_i64()
}

/// `["/bin/zsh", "-lc", "<script>"]` is the script; any other argv is its words.
fn argv_command(argv: &[Value]) -> String {
    const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];
    let words: Vec<&str> = argv.iter().filter_map(Value::as_str).collect();
    match words.as_slice() {
        [shell, "-lc" | "-c", script, ..]
            if Path::new(shell)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| SHELLS.contains(&n)) =>
        {
            (*script).to_string()
        }
        _ => words.join(" "),
    }
}

/// A shell command that applies a patch (`apply_patch <<'EOF' ...`) is a patch.
fn shell_or_patch(cmd: String) -> Tool {
    match cmd.find(PATCH_START) {
        Some(i) => Tool::Patch(cmd[i..].to_string()),
        None => Tool::Shell(cmd),
    }
}

fn tool_of(name: &str, namespace: Option<&str>, args: &Value) -> Tool {
    let text = |k: &str| args.get(k).and_then(Value::as_str).map(String::from);
    if namespace.is_some_and(|n| !n.is_empty()) {
        return Tool::Other;
    }
    match name {
        "exec_command" => text("cmd").map_or(Tool::Other, shell_or_patch),
        "shell_command" => text("command").map_or(Tool::Other, shell_or_patch),
        "shell" | "container.exec" => args
            .get("command")
            .and_then(Value::as_array)
            .map_or(Tool::Other, |a| shell_or_patch(argv_command(a))),
        "apply_patch" => text("input")
            .or_else(|| text("patch"))
            .or_else(|| text("command"))
            .map_or(Tool::Other, Tool::Patch),
        _ => Tool::Other,
    }
}

fn event_msg(p: &Value) -> Event {
    let kind = p.get("type").and_then(Value::as_str);
    let reply = |t: String| match non_empty(Some(&t)) {
        Some(t) => Event::Reply(t),
        None => Event::Other,
    };
    let prompt = |t: &str| prompt_text(t).map_or(Event::Other, Event::Prompt);
    match kind {
        Some("user_message") => prompt(p.get("message").and_then(Value::as_str).unwrap_or("")),
        Some("agent_message") => reply(joined(p.get("message"))),
        Some("item_completed") => {
            let item = p.get("item").unwrap_or(&Value::Null);
            let content = item.get("content");
            match item.get("type").and_then(Value::as_str) {
                Some("UserMessage") => prompt(&joined(content)),
                Some("AgentMessage") => reply(joined(content)),
                Some(kind) => tool_item(kind, item),
                None => Event::Other,
            }
        }
        Some("token_count") => p
            .pointer("/info/total_token_usage")
            .map_or(Event::Other, |u| Event::Total(Usage::of(u))),
        Some("context_compacted") => Event::Compacted,
        _ => Event::Other,
    }
}

/// A finished tool call of a paginated rollout.
fn tool_item(kind: &str, item: &Value) -> Event {
    let s = |k: &str| item.get(k).and_then(Value::as_str);
    let done = |tool, output: &str, exit| Event::Done {
        tool,
        output: output.to_string(),
        exit,
    };
    match kind {
        "CommandExecution" => {
            let cmd = match item.get("command") {
                Some(Value::Array(argv)) => argv_command(argv),
                Some(Value::String(cmd)) => cmd.clone(),
                _ => return Event::Other,
            };
            if cmd.trim().is_empty() {
                return Event::Other;
            }
            let exit = item.get("exit_code").and_then(Value::as_i64);
            done(Tool::Shell(cmd), s("aggregated_output").unwrap_or(""), exit)
        }
        "FileChange" => {
            let mut files = vec![];
            // a patch that failed or was declined changed nothing
            if !matches!(s("status"), Some("failed" | "declined")) {
                for (path, change) in item
                    .get("changes")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                {
                    files.push(path.clone());
                    files.extend(
                        change
                            .get("move_path")
                            .and_then(Value::as_str)
                            .map(String::from),
                    );
                }
            }
            done(Tool::Files(files), "", None)
        }
        "McpToolCall" => match (s("server"), s("tool")) {
            (Some(server), Some(tool)) => {
                done(Tool::Named(format!("mcp {server}.{tool}")), "", None)
            }
            _ => Event::Other,
        },
        "WebSearch" => done(
            Tool::Named(format!("web search {}", s("query").unwrap_or(""))),
            "",
            None,
        ),
        "DynamicToolCall" => {
            s("tool").map_or(Event::Other, |t| done(Tool::Named(t.into()), "", None))
        }
        _ => Event::Other,
    }
}

fn response_item(p: &Value) -> Event {
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let call_id = || s("call_id").unwrap_or("").to_string();
    match s("type") {
        Some("function_call") => {
            let args: Value = s("arguments")
                .and_then(|a| serde_json::from_str(a).ok())
                .unwrap_or(Value::Null);
            Event::Call {
                id: call_id(),
                tool: tool_of(s("name").unwrap_or(""), s("namespace"), &args),
            }
        }
        Some("custom_tool_call") => Event::Call {
            id: call_id(),
            tool: match (s("name"), s("input")) {
                (Some("apply_patch"), Some(patch)) => Tool::Patch(patch.to_string()),
                (Some("exec"), Some(js)) => Tool::Script(
                    script::calls(js)
                        .into_iter()
                        .map(|c| match c {
                            Call::Command(cmd) => shell_or_patch(cmd),
                            Call::Patch(patch) => Tool::Patch(patch),
                        })
                        .collect(),
                ),
                _ => Tool::Other,
            },
        },
        Some("local_shell_call") => Event::Call {
            id: call_id(),
            tool: p
                .pointer("/action/command")
                .and_then(Value::as_array)
                .map_or(Tool::Other, |a| shell_or_patch(argv_command(a))),
        },
        Some("web_search_call") => Event::Call {
            id: call_id(),
            tool: Tool::Other,
        },
        Some("function_call_output" | "custom_tool_call_output") => Event::Output {
            call_id: call_id(),
            text: joined(p.get("output")),
        },
        _ => Event::Other,
    }
}

/// Read one line of a rollout, whichever dialect it is in.
pub fn decode(v: &Value) -> Event {
    let null = Value::Null;
    let p = v.get("payload").unwrap_or(&null);
    match v.get("type").and_then(Value::as_str) {
        Some("session_meta") => Event::Meta {
            id: non_empty(p.get("id").and_then(Value::as_str)).map(|i| i.to_ascii_lowercase()),
            cwd: non_empty(p.get("cwd").and_then(Value::as_str)),
            branch: non_empty(p.pointer("/git/branch").and_then(Value::as_str))
                .filter(|b| b != "HEAD"),
        },
        Some("turn_context") => Event::Turn {
            cwd: non_empty(p.get("cwd").and_then(Value::as_str)),
        },
        Some("compacted") => Event::Compacted,
        Some("event_msg") => event_msg(p),
        // the item of a paginated rollout's tool call is its record; this is the model's raw copy
        Some("response_item") if v.get("ordinal").is_none_or(Value::is_null) => response_item(p),
        Some("token_usage_record") => p
            .get("thread_token_usage")
            .map_or(Event::Other, |u| Event::Total(Usage::of(u))),
        _ => Event::Other,
    }
}

/// The type of a line, read from its start before the line is parsed: the envelope has the
/// timestamp (and an ordinal) before `"type"`.
pub fn line_type(line: &[u8]) -> Option<&str> {
    const KEY: &[u8] = b"\"type\":\"";
    let head = &line[..line.len().min(200)];
    let at = head.windows(KEY.len()).position(|w| w == KEY)? + KEY.len();
    // a `"type"` after the payload begins is the payload's own, not the line's
    if head[..at].windows(9).any(|w| w == b"\"payload\"") {
        return None;
    }
    let rest = &head[at..];
    let end = rest.iter().position(|&b| b == b'"')?;
    std::str::from_utf8(&rest[..end]).ok()
}

/// The decoded line, None for one that is not a line of a kind eye reads.
pub fn read_line(line: &[u8]) -> Option<(Value, Event)> {
    match line_type(line) {
        Some(t) if !TYPES.contains(&t) => return None,
        None if line.len() > NO_TYPE_PARSE_MAX => return None,
        _ => {}
    }
    let v: Value = serde_json::from_slice(line).ok()?;
    let e = decode(&v);
    Some((v, e))
}

/// The files an `apply_patch` text adds, changes, deletes or moves to.
pub fn patch_files(patch: &str) -> Vec<&str> {
    const MARKS: [&str; 4] = [
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    patch
        .lines()
        .filter_map(|l| MARKS.iter().find_map(|m| l.strip_prefix(m)))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// `a/b/../c` as `a/c`, without asking the file system.
fn normalized(p: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// The folders a patch writes in, each once, for a command run in `cwd`.
pub fn patch_dirs(patch: &str, cwd: Option<&str>) -> Vec<String> {
    file_dirs(&patch_files(patch), cwd)
}

/// The folders of `files` (relative ones are taken against `cwd`), each once.
fn file_dirs(files: &[&str], cwd: Option<&str>) -> Vec<String> {
    let mut dirs: Vec<String> = vec![];
    for &f in files {
        let path = match (Path::new(f).is_absolute(), cwd) {
            (true, _) => PathBuf::from(f),
            (false, Some(c)) => Path::new(c).join(f),
            (false, None) => continue,
        };
        let Some(dir) = normalized(path)
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
        else {
            continue;
        };
        if !dir.is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// What a read hands to the next one: the folder and the token total it ended with, and the
/// `git commit` calls whose result has not been read yet.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Carry {
    cwd: Option<String>,
    total: Option<Usage>,
    commits: Vec<(String, Option<String>)>,
    /// The state of the background shells (see `background`).
    #[serde(default)]
    background: Option<Value>,
}

/// What reading a file found.
pub struct Parsed {
    pub delta: Delta,
    /// The read went on from an earlier one and met something it cannot add to that (the folder
    /// of the session changed, the token total went down): read the whole file again.
    pub restart: bool,
}

/// Turns the lines of one rollout into a [`Delta`].
pub struct Reader<'a> {
    d: Delta,
    seen: Seen,
    session_id: &'a str,
    ignore_dirs: &'a [String],
    resumed: bool,
    meta_seen: bool,
    cwd: Option<String>,
    before: Option<Usage>,
    total: Option<Usage>,
    commits: HashMap<String, Option<String>>,
    order: Vec<String>,
    background: Tracker,
    restart: bool,
}

impl<'a> Reader<'a> {
    /// A reader for the rollout of `session_id`. With `resumed` the earlier read's `carry` is
    /// what this one goes on from.
    pub fn new(session_id: &'a str, opts: &'a ParseOpts, resumed: bool) -> Reader<'a> {
        let carry: Carry = opts
            .carry
            .filter(|_| resumed)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let background = Tracker::resume(carry.background.as_ref());
        Reader {
            d: Delta::default(),
            seen: Seen::new(),
            session_id,
            ignore_dirs: opts.ignore_dirs,
            resumed,
            meta_seen: false,
            cwd: carry.cwd,
            before: carry.total,
            total: None,
            order: carry.commits.iter().map(|(k, _)| k.clone()).collect(),
            commits: carry.commits.into_iter().collect(),
            background,
            restart: false,
        }
    }

    /// Take in one line of the file.
    pub fn line(&mut self, line: &[u8]) {
        self.d.lines += 1;
        let Some((v, event)) = read_line(line) else {
            return;
        };
        let ts = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms);
        if let Some(t) = ts {
            self.d.first_ts = Some(self.d.first_ts.map_or(t, |x| x.min(t)));
            self.d.last_ts = Some(self.d.last_ts.map_or(t, |x| x.max(t)));
        }
        if let Some(e) = ts.and_then(|t| work_event(&event, t)) {
            self.d.events.push(e);
        }
        let ts = ts.unwrap_or(0);
        self.background.line(&v);
        match event {
            Event::Meta { id, cwd, branch } => self.meta(id, cwd, branch),
            Event::Turn { cwd: Some(c) } => {
                if self.resumed && self.cwd.as_deref().is_some_and(|x| x != c) {
                    self.restart = true;
                }
                self.cwd = Some(c);
            }
            Event::Prompt(p) => self.prompt(&p, ts),
            Event::Reply(t) => self.reply(&t, ts),
            Event::Call { id, tool } => self.call(id, tool, ts),
            Event::Output { call_id, text } => self.output(&call_id, &text, ts),
            Event::Done { tool, output, exit } => self.done(tool, &output, exit, ts),
            Event::Total(u) => self.total = Some(u),
            Event::Turn { cwd: None } | Event::Compacted | Event::Other => {}
        }
    }

    /// The session's own meta line (a legacy fork can carry its parent's too).
    fn meta(&mut self, id: Option<String>, cwd: Option<String>, branch: Option<String>) {
        if self.meta_seen || id.is_some_and(|i| i != self.session_id) {
            return;
        }
        self.meta_seen = true;
        if self.cwd.is_none() {
            self.cwd = cwd;
        }
        if let Some(b) = branch {
            scan_jira_branch(&mut self.d, &b);
            self.d.branch = Some(b);
        }
    }

    fn prompt(&mut self, p: &str, ts: i64) {
        let d = &mut self.d;
        d.prompt_count += 1;
        for pre in scan_jira(d, &mut self.seen, p, ts) {
            d.jira_prompt_prefixes.insert(pre);
        }
        let p = head_chars(p, MSG_CAP);
        if d.first_prompt.is_none() {
            d.first_prompt = Some(head_chars(&p, 2_000));
        }
        d.last_user_text = Some(head_chars(&p, 1_500));
        d.prompts_text.push_str(&p);
        d.prompts_text.push('\n');
        if d.prompts_text.len() > PROMPTS_CAP * 2 {
            trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
        }
    }

    fn reply(&mut self, t: &str, ts: i64) {
        let d = &mut self.d;
        d.reply_count += 1;
        scan_jira(d, &mut self.seen, t, ts);
        scan_pr(d, &mut self.seen, t, ts);
        d.last_assistant_text = Some(tail_chars(t, 1_500));
        d.replies_text.push_str(&head_chars(t, MSG_CAP));
        d.replies_text.push('\n');
        if d.replies_text.len() > REPLIES_CAP * 2 {
            trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
        }
    }

    /// A tool call whose result is still to come.
    fn call(&mut self, id: String, tool: Tool, ts: i64) {
        match tool {
            Tool::Shell(cmd) => {
                self.scan_command(&cmd, ts);
                if is_git_commit(&cmd) {
                    self.commits.insert(id.clone(), commit_message(&cmd));
                    self.order.push(id);
                }
            }
            Tool::Patch(patch) => self.touched(patch_dirs(&patch, self.cwd.as_deref())),
            Tool::Script(calls) => {
                for tool in calls {
                    self.call(id.clone(), tool, ts);
                }
            }
            Tool::Files(_) | Tool::Named(_) | Tool::Other => {}
        }
    }

    fn output(&mut self, call_id: &str, text: &str, ts: i64) {
        if let Some(message) = self.commits.remove(call_id) {
            self.committed(message, text, exit_code(text), ts);
        }
        self.scan_result(text, ts);
    }

    /// A tool call that comes with its result.
    fn done(&mut self, tool: Tool, output: &str, exit: Option<i64>, ts: i64) {
        match tool {
            Tool::Shell(cmd) => {
                self.scan_command(&cmd, ts);
                if is_git_commit(&cmd) {
                    self.committed(commit_message(&cmd), output, exit, ts);
                }
                self.scan_result(output, ts);
            }
            Tool::Files(files) => {
                let files: Vec<&str> = files.iter().map(String::as_str).collect();
                self.touched(file_dirs(&files, self.cwd.as_deref()));
            }
            Tool::Patch(_) | Tool::Script(_) | Tool::Named(_) | Tool::Other => {}
        }
    }

    /// What a command line mentions: Jira keys and sites.
    fn scan_command(&mut self, cmd: &str, ts: i64) {
        scan_jira(&mut self.d, &mut self.seen, cmd, ts);
        scan_jira_urls(&mut self.d, cmd);
    }

    /// What a command printed: pull requests and Jira sites.
    fn scan_result(&mut self, text: &str, ts: i64) {
        scan_pr(&mut self.d, &mut self.seen, text, ts);
        scan_jira_urls(&mut self.d, text);
    }

    /// A `git commit` that ended with `exit` (None: not said) and printed `text`.
    fn committed(&mut self, message: Option<String>, text: &str, exit: Option<i64>, ts: i64) {
        // Without a summary line (`git commit -q`) a command that exited 0 still committed.
        let sha = commit_sha(text);
        let committed = match exit {
            Some(code) => code == 0,
            None => sha.is_some(),
        };
        if committed {
            let value = sha.unwrap_or_else(|| message.clone().unwrap_or("commit".into()));
            push_output(
                &mut self.d,
                &mut self.seen,
                "commit",
                value,
                None,
                message,
                ts,
            );
        }
    }

    /// Folders a tool call wrote in.
    fn touched(&mut self, dirs: Vec<String>) {
        for dir in dirs {
            if !dir_ignored(&dir, self.ignore_dirs) {
                *self.d.folders.entry(dir).or_insert(0) += 1;
            }
        }
    }

    /// The delta of what was read; the file was read from `from` up to `end`.
    pub fn finish(mut self, from: u64, end: u64) -> Parsed {
        let mut restart = self.restart;
        self.d.usage = match (self.total, self.before) {
            (Some(now), Some(before)) => now.since(&before).map_or_else(
                || {
                    restart = true;
                    Tokens::default()
                },
                |added| added.tokens(),
            ),
            (Some(now), None) => now.tokens(),
            (None, _) => Tokens::default(),
        };
        self.order.retain(|k| self.commits.contains_key(k));
        let skip = self.order.len().saturating_sub(PENDING_CAP);
        self.d.signals = Some(self.background.signals());
        let carry = Carry {
            cwd: self.cwd.clone(),
            background: self.background.state(),
            total: self.total.or(self.before),
            commits: self
                .order
                .iter()
                .skip(skip)
                .filter_map(|k| Some((k.clone(), self.commits.get(k)?.clone())))
                .collect(),
        };
        let d = &mut self.d;
        d.carry = serde_json::to_value(carry).ok();
        d.session_id = Some(self.session_id.to_string());
        d.cwd = self.cwd;
        trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
        trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
        d.marker = end.to_string();
        d.bytes_read = end - from;
        Parsed {
            delta: self.d,
            restart,
        }
    }
}

/// Read the rollout at `path` from byte offset `from`. `resumed` says the read goes on from an
/// earlier one.
pub fn parse_file(
    path: &Path,
    from: u64,
    session_id: &str,
    opts: &ParseOpts,
    resumed: bool,
) -> Result<Parsed> {
    let mut r = Reader::new(session_id, opts, resumed);
    let end = for_each_line(path, from, |l| r.line(l))?;
    Ok(r.finish(from, end))
}

/// The thread's token total at the end of the rollout at `path` (a sub-agent's: nothing else is
/// read from it).
pub fn last_total(path: &Path) -> Result<Usage> {
    let mut total = Usage::default();
    for_each_line(path, 0, |line| {
        if !line.windows(11).any(|w| w == b"token_usage") {
            return;
        }
        if let Some((_, Event::Total(u))) = read_line(line) {
            total = u;
        }
    })?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;

    fn opts<'a>(job_tokens: &'a HashSet<String>) -> ParseOpts<'a> {
        ParseOpts {
            job_tokens,
            ignore_dirs: &[],
            since: None,
            carry: None,
            last_msg_id: None,
        }
    }

    fn read(lines: &[Value]) -> Parsed {
        let jt = HashSet::new();
        let o = opts(&jt);
        let mut r = Reader::new("s1", &o, false);
        for l in lines {
            r.line(l.to_string().as_bytes());
        }
        r.finish(0, 0)
    }

    fn ev(kind: &str, payload: Value) -> Value {
        json!({"timestamp": "2026-11-09T14:00:00.000Z", "type": kind, "payload": payload})
    }

    #[test]
    fn a_prompt_is_only_what_the_user_typed() {
        assert_eq!(prompt_text("  fix it  ").as_deref(), Some("fix it"));
        assert_eq!(
            prompt_text("# Context from my IDE setup:\n\n## Open tabs: a.py\n\n## My request for Codex:\nfix it")
                .as_deref(),
            Some("fix it")
        );
        for injected in [
            "# AGENTS.md instructions for /x\n\n<INSTRUCTIONS>...",
            "<environment_context>\n  <cwd>/x</cwd>",
            "<user_shell_command>\nls",
            "<turn_aborted>\nThe user interrupted",
            "<subagent_notification>{}",
            "<skill>\n<name>x</name>",
            "<model_switch>\nnew model",
            "<current_time_reminder>2026",
            "<codex_internal_context version=1>",
            "   ",
            "# Context from my IDE setup:\n\n## My request for Codex:\n",
        ] {
            assert_eq!(prompt_text(injected), None, "{injected}");
        }
    }

    #[test]
    fn both_dialects_give_the_same_events() {
        let legacy = decode(&ev(
            "event_msg",
            json!({"type": "user_message", "message": "hello", "kind": "plain"}),
        ));
        let paginated = decode(&ev(
            "event_msg",
            json!({"type": "item_completed", "turn_id": "t", "item": {"type": "UserMessage", "id": "u", "content": [{"type": "text", "text": "hello"}]}}),
        ));
        assert_eq!(legacy, Event::Prompt("hello".into()));
        assert_eq!(paginated, legacy);
        let legacy = decode(&ev(
            "event_msg",
            json!({"type": "agent_message", "message": "done"}),
        ));
        let paginated = decode(&ev(
            "event_msg",
            json!({"type": "item_completed", "item": {"type": "AgentMessage", "phase": "final_answer", "content": [{"type": "Text", "text": "done"}]}}),
        ));
        assert_eq!(legacy, Event::Reply("done".into()));
        assert_eq!(paginated, legacy);
        // the model's own copy of the conversation is not read for text
        for (role, kind) in [("user", "input_text"), ("assistant", "output_text")] {
            let e = decode(&ev(
                "response_item",
                json!({"type": "message", "role": role, "content": [{"type": kind, "text": "x"}]}),
            ));
            assert_eq!(e, Event::Other, "{role}");
        }
        // a prompt of images only is empty
        let e = decode(&ev(
            "event_msg",
            json!({"type": "user_message", "message": "", "images": ["data:image/png;base64,AAA"]}),
        ));
        assert_eq!(e, Event::Other);
    }

    #[test]
    fn tools_are_read_by_what_they_do_not_by_one_name() {
        let call = |name: &str, args: Value| {
            decode(&ev(
                "response_item",
                json!({"type": "function_call", "name": name, "arguments": args.to_string(), "call_id": "c"}),
            ))
        };
        let shell = |cmd: &str| Event::Call {
            id: "c".into(),
            tool: Tool::Shell(cmd.into()),
        };
        assert_eq!(
            call("exec_command", json!({"cmd": "ls -la"})),
            shell("ls -la")
        );
        assert_eq!(call("shell_command", json!({"command": "ls"})), shell("ls"));
        assert_eq!(
            call("shell", json!({"command": ["bash", "-lc", "git status"]})),
            shell("git status")
        );
        assert_eq!(
            call("shell", json!({"command": ["ls", "-l"]})),
            shell("ls -l")
        );
        assert!(matches!(
            call("exec_command", json!({"cmd": "apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: a\n*** End Patch\nEOF"})),
            Event::Call { tool: Tool::Patch(p), .. } if p.starts_with("*** Begin Patch")
        ));
        assert!(matches!(
            call(
                "apply_patch",
                json!({"input": "*** Begin Patch\n*** End Patch"})
            ),
            Event::Call {
                tool: Tool::Patch(_),
                ..
            }
        ));
        let mcp = decode(&ev(
            "response_item",
            json!({"type": "function_call", "name": "exec_command", "namespace": "mcp__x", "arguments": "{\"cmd\":\"ls\"}", "call_id": "c"}),
        ));
        assert_eq!(
            mcp,
            Event::Call {
                id: "c".into(),
                tool: Tool::Other
            }
        );
        let local = decode(&ev(
            "response_item",
            json!({"type": "local_shell_call", "call_id": "c", "status": "completed", "action": {"type": "exec", "command": ["bash", "-lc", "pwd"]}}),
        ));
        assert_eq!(local, shell("pwd"));
        let broken = decode(&ev(
            "response_item",
            json!({"type": "function_call", "name": "exec_command", "arguments": "{oops", "call_id": "c"}),
        ));
        assert_eq!(
            broken,
            Event::Call {
                id: "c".into(),
                tool: Tool::Other
            }
        );
    }

    /// A line of a paginated rollout: the envelope has an `ordinal`.
    fn pg(kind: &str, payload: Value) -> Value {
        json!({"timestamp": "2026-11-10T07:00:00.000Z", "ordinal": 3, "type": kind, "payload": payload})
    }

    fn item(item: Value) -> Value {
        pg(
            "event_msg",
            json!({"type": "item_completed", "turn_id": "t", "item": item}),
        )
    }

    #[test]
    fn tool_items_are_read_as_one_record_with_their_result() {
        let command = decode(&item(json!({
            "type": "CommandExecution", "command": ["/bin/zsh", "-lc", "sed -n '1,200p' calc.py"],
            "cwd": "file:///w/demo", "status": "completed", "aggregated_output": "def add(a, b):\n", "exit_code": 0
        })));
        assert_eq!(
            command,
            Event::Done {
                tool: Tool::Shell("sed -n '1,200p' calc.py".into()),
                output: "def add(a, b):\n".into(),
                exit: Some(0)
            }
        );
        let argv = decode(&item(
            json!({"type": "CommandExecution", "command": ["ls", "-l"]}),
        ));
        assert!(
            matches!(argv, Event::Done { tool: Tool::Shell(c), exit: None, .. } if c == "ls -l")
        );
        let files = decode(&item(json!({
            "type": "FileChange", "status": "completed",
            "changes": {"/w/a.py": {"type": "update", "unified_diff": "@@", "move_path": "/w/b.py"}, "/w/c.py": {"type": "add"}}
        })));
        assert_eq!(
            files,
            Event::Done {
                tool: Tool::Files(vec!["/w/a.py".into(), "/w/b.py".into(), "/w/c.py".into()]),
                output: String::new(),
                exit: None
            }
        );
        let declined = decode(&item(
            json!({"type": "FileChange", "status": "declined", "changes": {"/w/a.py": {"type": "add"}}}),
        ));
        assert!(matches!(declined, Event::Done { tool: Tool::Files(f), .. } if f.is_empty()));
        let named = |v: Value| match decode(&item(v)) {
            Event::Done {
                tool: Tool::Named(n),
                ..
            } => n,
            e => panic!("{e:?}"),
        };
        assert_eq!(
            named(
                json!({"type": "McpToolCall", "server": "jira", "tool": "get_issue", "arguments": {}})
            ),
            "mcp jira.get_issue"
        );
        assert_eq!(
            named(
                json!({"type": "WebSearch", "query": "rust flock", "action": {"type": "search"}})
            ),
            "web search rust flock"
        );
        assert_eq!(
            decode(&item(json!({"type": "Reasoning", "summary": []}))),
            Event::Other
        );
        assert_eq!(
            decode(&item(json!({"type": "CommandExecution", "command": []}))),
            Event::Other
        );
    }

    #[test]
    fn a_shell_wrapper_is_taken_off_only_for_a_shell() {
        let argv = |w: &[&str]| argv_command(&w.iter().map(|s| json!(s)).collect::<Vec<_>>());
        assert_eq!(argv(&["/bin/zsh", "-lc", "git status"]), "git status");
        assert_eq!(argv(&["bash", "-c", "ls", "arg0"]), "ls");
        assert_eq!(
            argv(&["python3", "-c", "print('git commit')"]),
            "python3 -c print('git commit')"
        );
        assert_eq!(argv(&["ls", "-la"]), "ls -la");
    }

    #[test]
    fn a_paginated_tool_call_counts_once_though_the_file_records_it_twice() {
        // the raw call and its output (what the model saw) next to the item that says what ran
        let p = read(&[
            pg(
                "response_item",
                json!({"type": "function_call", "name": "exec_command", "call_id": "c",
                       "arguments": json!({"cmd": "git commit -m \"Add it\""}).to_string()}),
            ),
            pg(
                "response_item",
                json!({"type": "function_call_output", "call_id": "c", "output": "Process exited with code 0\nOutput:\n[main 1a2b3c4] Add it\n"}),
            ),
            item(
                json!({"type": "CommandExecution", "command": ["/bin/zsh", "-lc", "git commit -m \"Add it\""],
                        "status": "completed", "aggregated_output": "[main 1a2b3c4] Add it\n", "exit_code": 0}),
            ),
            pg(
                "response_item",
                json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p",
                       "input": "*** Begin Patch\n*** Add File: src/a.rs\n*** End Patch"}),
            ),
            item(
                json!({"type": "FileChange", "status": "completed", "changes": {"/w/app/src/a.rs": {"type": "add"}}}),
            ),
        ]);
        let outs: Vec<(&str, &str)> = p
            .delta
            .outputs
            .iter()
            .map(|o| (o.kind, o.value.as_str()))
            .collect();
        assert_eq!(outs, [("commit", "1a2b3c4")]);
        assert_eq!(p.delta.folders.get("/w/app/src"), Some(&1));
        assert_eq!(p.delta.folders.len(), 1);
    }

    #[test]
    fn what_a_paginated_command_printed_is_scanned_and_a_failed_commit_is_not_one() {
        let run = |cmd: &str, out: &str, exit: i64| {
            item(
                json!({"type": "CommandExecution", "command": ["/bin/zsh", "-lc", cmd],
                        "status": "completed", "aggregated_output": out, "exit_code": exit}),
            )
        };
        let p = read(&[
            run("git commit -m \"Nothing\"", "nothing to commit\n", 1),
            run("git commit -q -m \"Quiet\"", "", 0),
            run(
                "gh pr create --fill",
                "https://github.com/org/app/pull/42\n",
                0,
            ),
            run("git push && echo git commit", "Everything up-to-date\n", 0),
        ]);
        let got: Vec<(&str, &str)> = p
            .delta
            .outputs
            .iter()
            .map(|o| (o.kind, o.value.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("commit", "Quiet"),
                ("pr", "https://github.com/org/app/pull/42")
            ]
        );
    }

    #[test]
    fn a_code_mode_program_of_a_rollout_without_items_is_read_for_what_it_runs() {
        let exec = |id: &str, js: &str| {
            ev(
                "response_item",
                json!({"type": "custom_tool_call", "name": "exec", "call_id": id, "input": js}),
            )
        };
        let result = |id: &str, text: &str| {
            ev(
                "response_item",
                json!({"type": "custom_tool_call_output", "call_id": id, "output": [{"type": "input_text", "text": text}]}),
            )
        };
        let p = read(&[
            ev("turn_context", json!({"cwd": "/w/app"})),
            exec(
                "1",
                r#"const r = await tools.exec_command({cmd:"git commit -m \"Add it\""}); text(r.output);"#,
            ),
            result("1", "Script completed\nOutput:\n[main 1a2b3c4] Add it\n"),
            exec(
                "2",
                r#"text(await tools.apply_patch("*** Begin Patch\n*** Update File: src/a.rs\n*** End Patch"))"#,
            ),
            exec("3", "const cmd = pick(); await tools.exec_command({cmd});"),
        ]);
        let outs: Vec<(&str, &str)> = p
            .delta
            .outputs
            .iter()
            .map(|o| (o.kind, o.value.as_str()))
            .collect();
        assert_eq!(outs, [("commit", "1a2b3c4")]);
        assert_eq!(p.delta.folders.get("/w/app/src"), Some(&1));
        let unreadable = decode(&exec(
            "3",
            "const cmd = pick(); await tools.exec_command({cmd});",
        ));
        assert_eq!(
            unreadable,
            Event::Call {
                id: "3".into(),
                tool: Tool::Script(vec![])
            }
        );
    }

    #[test]
    fn a_legacy_patch_writes_where_the_turn_was_and_an_item_writes_where_it_says() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n*** End Patch";
        let legacy = read(&[
            ev("turn_context", json!({"cwd": "/w/app"})),
            ev(
                "response_item",
                json!({"type": "custom_tool_call", "name": "apply_patch", "call_id": "p", "input": patch}),
            ),
        ]);
        assert_eq!(legacy.delta.folders.get("/w/app/src"), Some(&1));
        let paginated = read(&[
            pg("turn_context", json!({"cwd": "/w/app"})),
            item(json!({"type": "FileChange", "changes": {"/w/other/b.rs": {"type": "update"}}})),
        ]);
        assert_eq!(paginated.delta.folders.get("/w/other"), Some(&1));
        assert_eq!(paginated.delta.folders.len(), 1);
    }

    #[test]
    fn a_command_result_carries_its_exit_code_in_either_form() {
        assert_eq!(
            exit_code(
                "Chunk ID: a\nWall time: 0.1 seconds\nProcess exited with code 2\nOutput:\nboom"
            ),
            Some(2)
        );
        assert_eq!(
            exit_code(r#"{"output":"x","metadata":{"exit_code":1,"duration_seconds":0.1}}"#),
            Some(1)
        );
        assert_eq!(
            exit_code("Chunk ID: a\nProcess running with session ID 7\nOutput:\nProcess exited with code 9"),
            None,
            "the words in the output are not the exit code"
        );
        assert_eq!(exit_code("plain text"), None);
    }

    #[test]
    fn the_total_is_the_last_one_and_never_a_sum() {
        let u = |i, c, o| json!({"input_tokens": i, "cached_input_tokens": c, "cache_write_input_tokens": 0, "output_tokens": o, "reasoning_output_tokens": 0, "total_tokens": i + o});
        let count = |total: Value| {
            ev(
                "event_msg",
                json!({"type": "token_count", "info": {"total_token_usage": total, "last_token_usage": u(1, 0, 1), "model_context_window": 1}}),
            )
        };
        let p = read(&[
            ev("event_msg", json!({"type": "token_count", "info": null})),
            count(u(100, 40, 10)),
            count(u(300, 200, 30)),
            ev("event_msg", json!({"type": "token_count", "info": null})),
        ]);
        let t = p.delta.usage;
        assert_eq!((t.input, t.cache_read, t.output), (100, 200, 30));
        assert!(!p.restart);
    }

    #[test]
    fn a_token_total_that_went_down_asks_for_a_fresh_read() {
        let carry = serde_json::to_value(Carry {
            cwd: Some("/x".into()),
            total: Some(Usage {
                input: 500,
                cached: 0,
                cache_write: 0,
                output: 50,
            }),
            commits: vec![],
            background: None,
        })
        .unwrap();
        let jt = HashSet::new();
        let mut o = opts(&jt);
        o.carry = Some(&carry);
        let mut r = Reader::new("s1", &o, true);
        r.line(
            ev(
                "event_msg",
                json!({"type": "token_count", "info": {"total_token_usage": {"input_tokens": 10, "cached_input_tokens": 0, "cache_write_input_tokens": 0, "output_tokens": 1, "reasoning_output_tokens": 0, "total_tokens": 11}}}),
            )
            .to_string()
            .as_bytes(),
        );
        assert!(r.finish(0, 0).restart);
    }

    #[test]
    fn only_a_commit_that_succeeded_is_an_output() {
        let call = |id: &str, cmd: &str| {
            ev(
                "response_item",
                json!({"type": "function_call", "name": "exec_command", "arguments": json!({"cmd": cmd}).to_string(), "call_id": id}),
            )
        };
        let out = |id: &str, text: &str| {
            ev(
                "response_item",
                json!({"type": "function_call_output", "call_id": id, "output": text}),
            )
        };
        let p = read(&[
            call("1", "git commit -m \"Add it\""),
            out(
                "1",
                "Process exited with code 0\nOutput:\n[main 1a2b3c4] Add it\n",
            ),
            call("2", "git commit -m \"Nothing\""),
            out(
                "2",
                "Process exited with code 1\nOutput:\nnothing to commit\n",
            ),
            call("3", "git commit -q -m \"Quiet\""),
            out("3", "Process exited with code 0\nOutput:\n"),
            call("4", "echo git commit"),
            out("4", "Process exited with code 0\nOutput:\ngit commit\n"),
        ]);
        let got: Vec<(&str, &str)> = p
            .delta
            .outputs
            .iter()
            .map(|o| (o.kind, o.value.as_str()))
            .collect();
        assert_eq!(got, [("commit", "1a2b3c4"), ("commit", "Quiet")]);
    }

    #[test]
    fn a_patch_writes_in_the_folders_of_its_files() {
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: src/b.rs\n+z\n*** Delete File: docs/c.md\n*** Update File: /abs/d.rs\n*** Move to: ../other/e.rs\n*** End Patch";
        assert_eq!(
            patch_dirs(patch, Some("/w/app")),
            ["/w/app/src", "/w/app/docs", "/abs", "/w/other"]
        );
        assert_eq!(patch_dirs(patch, None), ["/abs"]);
        assert!(patch_dirs("no files here", Some("/w")).is_empty());
    }

    #[test]
    fn a_forks_inherited_meta_does_not_give_the_folder() {
        let meta = |id: &str, cwd: &str| {
            ev(
                "session_meta",
                json!({"id": id, "cwd": cwd, "git": {"branch": "PROJ-5-x"}}),
            )
        };
        let p = read(&[meta("PARENT", "/parent"), meta("S1", "/mine")]);
        assert_eq!(p.delta.cwd.as_deref(), Some("/mine"));
        assert_eq!(p.delta.branch.as_deref(), Some("PROJ-5-x"));
        assert!(p.delta.jira_branch_prefixes.contains("PROJ"));
        let head = read(&[ev(
            "session_meta",
            json!({"id": "S1", "cwd": "/w", "git": {"branch": "HEAD"}}),
        )]);
        assert_eq!(head.delta.branch, None);
    }

    #[test]
    fn lines_of_kinds_eye_does_not_read_are_skipped_before_parsing() {
        assert_eq!(
            line_type(br#"{"timestamp":"2026-11-09T14:00:00.000Z","ordinal":7,"type":"world_state","payload":{}}"#),
            Some("world_state")
        );
        assert_eq!(line_type(b"not a line"), None);
        // if a later version puts the payload first, the type inside it is not the line's
        assert_eq!(
            line_type(br#"{"timestamp":"t","payload":{"type":"message","role":"user"},"type":"response_item"}"#),
            None
        );
        let swapped = br#"{"payload":{"type":"user_message","message":"hi"},"type":"event_msg","timestamp":"t"}"#;
        assert_eq!(
            read_line(swapped).map(|(_, e)| e),
            Some(Event::Prompt("hi".into()))
        );
        assert!(read_line(
            br#"{"timestamp":"t","type":"world_state","payload":{"type":"event_msg"}}"#
        )
        .is_none());
        assert!(read_line(br#"{"timestamp":"t","type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#).is_some());
        let p = read(&[json!({"type": "world_state", "payload": {"x": 1}})]);
        assert_eq!(p.delta.lines, 1);
    }
}
