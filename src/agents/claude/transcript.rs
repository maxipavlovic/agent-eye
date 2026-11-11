//! Claude Code transcripts (`projects/<dir>/<session>.jsonl`): a streaming parser that turns
//! `(reader, from_offset)` into a [`Delta`] of what is new.

use super::super::append_only::for_each_line;
use super::super::outputs::{
    cap_str, commit_message, commit_sha, dir_ignored, is_git_commit, push_output, scan_jira,
    scan_jira_branch, scan_jira_urls, scan_pr, Seen, RESULT_SCAN_CAP,
};
use super::super::{Delta, ParseOpts, PROMPTS_CAP, REPLIES_CAP};
use super::signals::Tracker;
use crate::model::Tokens;
use crate::util::{head_chars, iso_to_ms, tail_chars, trim_front_bytes};
use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

const MSG_CAP: usize = 12_000;
const CHILD_SCAN_CAP: usize = 4_096;
const NO_TYPE_PARSE_MAX: usize = 64 << 20;

static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<system-reminder>.*?</system-reminder>|<local-command-caveat>.*?</local-command-caveat>|<local-command-stdout>.*?</local-command-stdout>|<local-command-stderr>.*?</local-command-stderr>")
        .expect("static regex")
});
static RE_CMD_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<command-name>(.*?)</command-name>").expect("static regex"));
static RE_CMD_ARGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<command-args>(.*?)</command-args>").expect("static regex"));
static RE_ARTIFACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://claude\.ai/(?:code/)?artifact/[A-Za-z0-9-]{8,}").expect("static regex")
});
static RE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[0-9a-f]{8}(?:-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})?\b")
        .expect("static regex")
});

const PENDING_CAP: usize = 64;

/// A tool_use of interest whose tool_result has not been read yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pending {
    Artifact {
        title: Option<String>,
        input_url: Option<String>,
    },
    Commit {
        message: Option<String>,
    },
}

/// Parse the transcript at `path` from byte offset `from`; the marker of the result is the offset
/// after the last complete line.
pub fn parse_file(path: &Path, from: u64, opts: &ParseOpts) -> Result<Delta> {
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    f.seek(SeekFrom::Start(from))?;
    parse_reader(BufReader::with_capacity(1 << 20, f), from, opts)
}

/// Parse transcript lines from `r`, which starts at byte offset `from` of its file.
pub fn parse_reader<R: BufRead>(mut r: R, from: u64, opts: &ParseOpts) -> Result<Delta> {
    let mut d = Delta {
        last_msg_id: opts.last_msg_id.map(String::from),
        ..Default::default()
    };
    let mut offset = from;
    let (pending_in, signals_in) = split_carry(opts.carry);
    let mut tracker = Tracker::resume(signals_in);
    let mut pending: HashMap<String, Pending> = pending_in.iter().cloned().collect();
    let mut order: Vec<String> = pending_in.iter().map(|(k, _)| k.clone()).collect();
    let mut seen = Seen::new();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let n = r.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if buf.last() != Some(&b'\n') {
            // half-written last line: leave it for the next scan
            break;
        }
        offset += n as u64;
        d.lines += 1;
        let line = &buf[..n - 1];
        let ty = match line_type(line) {
            Some(t) => t,
            None if line.len() <= NO_TYPE_PARSE_MAX => "",
            None => continue,
        };
        // Whitelist: everything else (attachments, api-request*, file-history*, ...) is skipped
        // before JSON parsing. They embed MEMORY.md and tool lists and would give false links.
        if !matches!(
            ty,
            "user"
                | "assistant"
                | "system"
                | "ai-title"
                | "custom-title"
                | "agent-name"
                | "queue-operation"
                | ""
        ) {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let ty = if ty.is_empty() {
            v.get("type").and_then(Value::as_str).unwrap_or("")
        } else {
            ty
        };
        tracker.line(&v, ty);
        process(&v, ty, &mut d, &mut pending, &mut seen, opts);
        if pending.len() > order.len() || order.len() > PENDING_CAP * 4 {
            order.retain(|k| pending.contains_key(k));
            for k in pending.keys() {
                if !order.contains(k) {
                    order.push(k.clone());
                }
            }
        }
    }
    order.retain(|k| pending.contains_key(k));
    let skip = order.len().saturating_sub(PENDING_CAP);
    let mut pending_out: Vec<(String, Pending)> = vec![];
    for k in order.into_iter().skip(skip) {
        if let Some(p) = pending.remove(&k) {
            pending_out.push((k, p));
        }
    }
    d.signals = Some(tracker.signals());
    let signals_out = tracker.state();
    if !pending_out.is_empty() || signals_out.is_some() {
        d.carry = Some(serde_json::json!({"pending": pending_out, "signals": signals_out}));
    }
    trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
    trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
    d.marker = offset.to_string();
    d.bytes_read = offset - from;
    Ok(d)
}

/// What an earlier read carried: the tool calls whose result is still to come, and the state of the
/// background tasks and loops. Before the second existed the carry was the list alone.
fn split_carry(carry: Option<&Value>) -> (Vec<(String, Pending)>, Option<&Value>) {
    let pending = |v: &Value| serde_json::from_value(v.clone()).unwrap_or_default();
    match carry {
        Some(v) if v.is_array() => (pending(v), None),
        Some(v) => (
            v.get("pending").map(pending).unwrap_or_default(),
            v.get("signals"),
        ),
        None => (vec![], None),
    }
}

/// The type of lines that start with `{"type":"..."` (attachments, titles, api-request, ...).
/// user/assistant/system lines start with other keys (the type sits after the message), so
/// they return `None` and are parsed.
fn line_type(line: &[u8]) -> Option<&str> {
    let rest = line.strip_prefix(b"{\"type\":\"")?;
    let end = rest.iter().take(64).position(|&b| b == b'"')?;
    std::str::from_utf8(&rest[..end]).ok()
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn process(
    v: &Value,
    ty: &str,
    d: &mut Delta,
    pending: &mut HashMap<String, Pending>,
    seen: &mut Seen,
    opts: &ParseOpts,
) {
    match ty {
        "queue-operation" => return,
        "ai-title" => {
            d.ai_title = s(v, "aiTitle")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            return;
        }
        "custom-title" => {
            d.custom_title = s(v, "customTitle")
                .map(|t| {
                    t.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace())
                        .to_string()
                })
                .filter(|t| !t.is_empty());
            return;
        }
        "agent-name" => {
            d.agent_title = s(v, "agentName")
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            return;
        }
        _ => {}
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let ts = s(v, "timestamp").and_then(iso_to_ms);
    if let Some(t) = ts {
        d.first_ts = Some(d.first_ts.map_or(t, |x| x.min(t)));
        d.last_ts = Some(d.last_ts.map_or(t, |x| x.max(t)));
    }
    let ts = ts.unwrap_or(0);
    if d.session_id.is_none() {
        d.session_id = s(v, "sessionId").map(String::from);
    }
    if d.cwd.is_none() {
        d.cwd = s(v, "cwd").filter(|c| !c.is_empty()).map(String::from);
    }
    if let Some(b) = s(v, "gitBranch") {
        if !b.is_empty() && b != "HEAD" {
            d.branch = Some(b.to_string());
            scan_jira_branch(d, b);
        }
    }
    match ty {
        "system" => {
            if s(v, "sessionKind") == Some("bg") {
                d.bg = true;
            }
            if s(v, "subtype") == Some("away_summary") {
                if let Some(c) = s(v, "content").filter(|c| !c.trim().is_empty()) {
                    if d.recap.as_ref().is_none_or(|(t, _)| ts >= *t) {
                        d.recap = Some((ts, c.trim().to_string()));
                    }
                }
            }
        }
        "user" => process_user(v, d, pending, seen, opts, ts),
        "assistant" => process_assistant(v, d, pending, seen, opts, ts),
        _ => {}
    }
}

/// The text of a user line as a prompt: injected tags removed, commands as `/name args`; None when
/// the line is not a prompt (task notifications, empty text).
pub fn clean_prompt(raw: &str) -> Option<String> {
    // Claude Code delivers background-task results as user lines; they are not prompts.
    if raw.trim_start().starts_with("<task-notification>") {
        return None;
    }
    if let Some(c) = RE_CMD_NAME.captures(raw) {
        let name = c.get(1).map_or("", |m| m.as_str()).trim();
        let args = RE_CMD_ARGS
            .captures(raw)
            .and_then(|a| a.get(1).map(|m| m.as_str().trim().to_string()))
            .unwrap_or_default();
        if name.is_empty() {
            return None;
        }
        return Some(format!("{name} {args}").trim().to_string());
    }
    let t = RE_TAGS.replace_all(raw, "");
    let t = t.trim();
    if t.is_empty() || t.starts_with("[Request interrupted") {
        return None;
    }
    Some(t.to_string())
}

pub(super) fn result_text(content: Option<&Value>) -> String {
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

fn process_user(
    v: &Value,
    d: &mut Delta,
    pending: &mut HashMap<String, Pending>,
    seen: &mut Seen,
    opts: &ParseOpts,
    ts: i64,
) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return;
    };
    let is_meta = v.get("isMeta").and_then(Value::as_bool) == Some(true)
        || v.get("isCompactSummary").and_then(Value::as_bool) == Some(true);
    let mut texts: Vec<&str> = Vec::new();
    match content {
        Value::String(t) => texts.push(t),
        Value::Array(blocks) => {
            for b in blocks {
                match s(b, "type") {
                    Some("tool_result") => handle_result(b, d, pending, seen, opts, ts),
                    Some("text") => {
                        if let Some(t) = s(b, "text") {
                            texts.push(t);
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if is_meta || texts.is_empty() {
        return;
    }
    let Some(p) = clean_prompt(&texts.join("\n")) else {
        return;
    };
    d.prompt_count += 1;
    for pre in scan_jira(d, seen, &p, ts) {
        d.jira_prompt_prefixes.insert(pre);
    }
    let p = head_chars(&p, MSG_CAP);
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

fn handle_result(
    b: &Value,
    d: &mut Delta,
    pending: &mut HashMap<String, Pending>,
    seen: &mut Seen,
    opts: &ParseOpts,
    ts: i64,
) {
    let text = result_text(b.get("content"));
    let is_err = b.get("is_error").and_then(Value::as_bool) == Some(true);
    if let Some(p) = s(b, "tool_use_id").and_then(|id| pending.remove(id)) {
        if !is_err {
            match p {
                Pending::Artifact { title, input_url } => {
                    let url = RE_ARTIFACT
                        .find(cap_str(&text, RESULT_SCAN_CAP))
                        .map(|m| m.as_str().to_string())
                        .or(input_url);
                    if let Some(u) = url {
                        push_output(d, seen, "artifact", u.clone(), Some(u), title, ts);
                    }
                }
                Pending::Commit { message } => {
                    // RTK rewrites the output to "ok 1 file changed", so the sha is optional.
                    let sha = commit_sha(&text);
                    let value = sha.unwrap_or_else(|| message.clone().unwrap_or("commit".into()));
                    push_output(d, seen, "commit", value, None, message, ts);
                }
            }
        }
    }
    scan_pr(d, seen, &text, ts);
    scan_jira_urls(d, &text);
    if !opts.job_tokens.is_empty() {
        for m in RE_TOKEN.find_iter(cap_str(&text, CHILD_SCAN_CAP)) {
            if opts.job_tokens.contains(m.as_str()) {
                d.child_hits.insert(m.as_str().to_string());
            }
        }
    }
}

/// Add the usage of an assistant line. Claude Code writes one line per content block of a
/// message and repeats the same usage on each, so a message id counts once.
pub fn add_usage(v: &Value, total: &mut Tokens, last_id: &mut Option<String>) {
    let Some(m) = v.get("message") else {
        return;
    };
    let Some(u) = m.get("usage") else {
        return;
    };
    if let Some(id) = s(m, "id") {
        if last_id.as_deref() == Some(id) {
            return;
        }
        *last_id = Some(id.to_string());
    }
    let n = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0).max(0);
    total.input += n("input_tokens");
    total.output += n("output_tokens");
    total.cache_write += n("cache_creation_input_tokens");
    total.cache_read += n("cache_read_input_tokens");
}

/// Token use of a transcript from `from` on (sub-agent transcripts: nothing else is read).
/// Returns the counts, the last message id counted and the offset after the last complete line.
pub fn parse_usage_file(
    path: &Path,
    from: u64,
    last_id: Option<String>,
) -> Result<(Tokens, Option<String>, u64)> {
    let (mut total, mut last) = (Tokens::default(), last_id);
    let offset = for_each_line(path, from, |line| {
        if !line.windows(7).any(|w| w == b"\"usage\"") {
            return;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if v.get("type").and_then(Value::as_str) == Some("assistant") {
            add_usage(&v, &mut total, &mut last);
        }
    })?;
    Ok((total, last, offset))
}

fn process_assistant(
    v: &Value,
    d: &mut Delta,
    pending: &mut HashMap<String, Pending>,
    seen: &mut Seen,
    opts: &ParseOpts,
    ts: i64,
) {
    add_usage(v, &mut d.usage, &mut d.last_msg_id);
    let Some(Value::Array(blocks)) = v.get("message").and_then(|m| m.get("content")) else {
        return;
    };
    let mut texts: Vec<&str> = Vec::new();
    for b in blocks {
        match s(b, "type") {
            Some("text") => {
                if let Some(t) = s(b, "text").filter(|t| !t.trim().is_empty()) {
                    texts.push(t);
                }
            }
            Some("tool_use") => handle_tool_use(b, d, pending, seen, opts, ts),
            _ => {}
        }
    }
    if texts.is_empty() {
        return;
    }
    let t = texts.join("\n");
    d.reply_count += 1;
    scan_jira(d, seen, &t, ts);
    scan_pr(d, seen, &t, ts);
    d.last_assistant_text = Some(tail_chars(&t, 1_500));
    let t = head_chars(&t, MSG_CAP);
    d.replies_text.push_str(&t);
    d.replies_text.push('\n');
    if d.replies_text.len() > REPLIES_CAP * 2 {
        trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
    }
}

/// File stems that say nothing about the content; the folder name is used instead.
const GENERIC_STEMS: &[&str] = &[
    "index",
    "main",
    "page",
    "app",
    "artifact",
    "report",
    "draft",
    "doc",
    "document",
    "output",
    "result",
    "readme",
    "new",
    "untitled",
    "test",
    "tmp",
    "temp",
    "dashboard",
    "slides",
    "deck",
];

/// A title from an artifact file path: the file stem, or the parent folder when the stem is
/// generic ("…/proposal/index.html" gives "proposal").
fn path_title(p: &str) -> Option<String> {
    let path = Path::new(p);
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    if !GENERIC_STEMS.contains(&stem.to_lowercase().as_str()) {
        return Some(stem);
    }
    let parent = path
        .parent()
        .and_then(Path::file_name)
        .map(|x| x.to_string_lossy().into_owned())
        .filter(|x| !x.is_empty() && x != "~");
    Some(parent.unwrap_or(stem))
}

fn handle_tool_use(
    b: &Value,
    d: &mut Delta,
    pending: &mut HashMap<String, Pending>,
    seen: &mut Seen,
    opts: &ParseOpts,
    ts: i64,
) {
    let (Some(name), Some(id)) = (s(b, "name"), s(b, "id")) else {
        return;
    };
    let input = b.get("input").cloned().unwrap_or(Value::Null);
    match name {
        "Artifact" => {
            // Only a publish makes an artifact; quickstart, read and list replies name other ones.
            if !matches!(s(&input, "action"), None | Some("publish")) {
                return;
            }
            let title = s(&input, "title")
                .filter(|t| !t.trim().is_empty())
                .map(String::from)
                .or_else(|| s(&input, "file_path").and_then(path_title));
            let input_url = s(&input, "url")
                .filter(|u| RE_ARTIFACT.is_match(u))
                .map(String::from);
            pending.insert(id.to_string(), Pending::Artifact { title, input_url });
        }
        "Bash" => {
            if let Some(cmd) = s(&input, "command") {
                scan_jira(d, seen, cmd, ts);
                scan_jira_urls(d, cmd);
                if is_git_commit(cmd) {
                    pending.insert(
                        id.to_string(),
                        Pending::Commit {
                            message: commit_message(cmd),
                        },
                    );
                }
            }
        }
        "Write" | "Edit" | "NotebookEdit" => {
            let p = s(&input, "file_path").or_else(|| s(&input, "notebook_path"));
            if let Some(dir) = p.and_then(|p| Path::new(p).parent()) {
                let dir = dir.to_string_lossy();
                if !dir.is_empty() && !dir_ignored(&dir, opts.ignore_dirs) {
                    *d.folders.entry(dir.into_owned()).or_insert(0) += 1;
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn task_notifications_are_not_prompts() {
        assert_eq!(
            clean_prompt("<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>"),
            None
        );
        assert_eq!(
            clean_prompt("fix the login bug").as_deref(),
            Some("fix the login bug")
        );
    }
    use std::io::Cursor;

    fn parse(bytes: &[u8]) -> Delta {
        let tokens: HashSet<String> = ["beef0001".to_string()].into();
        parse_reader(
            Cursor::new(bytes.to_vec()),
            0,
            &ParseOpts {
                job_tokens: &tokens,
                ignore_dirs: &[],
                since: None,
                carry: None,
                last_msg_id: None,
            },
        )
        .unwrap()
    }

    #[test]
    fn full_fixture() {
        let bytes = include_bytes!("../../../tests/fixtures/full.jsonl");
        let d = parse(bytes);
        assert_eq!(
            d.session_id.as_deref(),
            Some("aaaaaaaa-0000-4000-8000-000000000001")
        );
        assert_eq!(d.cwd.as_deref(), Some("/tmp/demo"));
        assert_eq!(d.branch.as_deref(), Some("feature/x"));
        // meta + tool_result + reminder-only lines are not prompts; the slash command is
        assert_eq!(d.prompt_count, 2, "{:?}", d.prompts_text);
        assert!(d.prompts_text.contains("migrate the catalog"));
        assert!(!d.prompts_text.contains("system-reminder"));
        assert!(!d.prompts_text.contains("PROJ-11111"));
        assert!(d.prompts_text.contains("/rename foo"));
        assert_eq!(
            d.first_prompt.as_deref(),
            Some("Please migrate the catalog widget, see PROJ-12345")
        );
        assert_eq!(d.custom_title.as_deref(), Some("My custom"));
        assert_eq!(d.ai_title.as_deref(), Some("AI title"));
        assert_eq!(d.agent_title.as_deref(), Some("agent-x"));
        let r = d.recap.as_ref().unwrap();
        assert!(r.1.starts_with("Goal: migrate"));
        let kinds = |k: &str| -> Vec<String> {
            d.outputs
                .iter()
                .filter(|o| o.kind == k)
                .map(|o| o.value.clone())
                .collect()
        };
        assert_eq!(
            kinds("artifact"),
            vec!["https://claude.ai/artifact/abc12345-def"]
        );
        assert_eq!(
            d.outputs
                .iter()
                .find(|o| o.kind == "artifact")
                .unwrap()
                .title
                .as_deref(),
            Some("Catalog plan")
        );
        assert_eq!(kinds("commit"), vec!["1a2b3c4"]);
        let prs = kinds("pr");
        assert!(prs.contains(&"https://bitbucket.org/acme/repo/pull-requests/61".to_string()));
        assert!(prs.contains(&"https://github.com/acme/repo/pull/9".to_string()));
        assert!(
            prs.contains(&"https://github.com/acme/repo/pull/10".to_string()),
            "from tool result"
        );
        assert_eq!(
            prs.len(),
            3,
            "attachment lines must not produce outputs: {prs:?}"
        );
        let jira: Vec<String> = d.jira.iter().map(|(k, _)| k.clone()).collect();
        assert!(jira.contains(&"PROJ-12345".to_string()));
        assert!(jira.contains(&"ZED-77".to_string()), "from Bash command");
        assert!(!jira.contains(&"PROJ-11111".to_string()));
        assert!(!jira.contains(&"PROJ-99999".to_string()), "attachment line");
        assert!(d.jira_prompt_prefixes.contains("PROJ"));
        assert!(!jira
            .iter()
            .any(|j| j.starts_with("UTF") || j.starts_with("SHA")));
        assert_eq!(d.folders.get("/tmp/demo/src"), Some(&2));
        assert!(d.child_hits.contains("beef0001"));
        assert!(d.last_assistant_text.as_deref().unwrap().ends_with('?'));
        assert!(d.bg);
        assert!(d.reply_count >= 3);
    }

    #[test]
    fn truncated_last_line_is_not_consumed() {
        let a = b"{\"type\":\"user\",\"timestamp\":\"2026-11-01T10:00:00.000Z\",\"message\":{\"content\":\"one\"}}\n";
        let mut bytes = a.to_vec();
        bytes.extend_from_slice(b"{\"type\":\"user\",\"message\":{\"content\":\"tw");
        let d = parse(&bytes);
        assert_eq!(d.prompt_count, 1);
        assert_eq!(d.marker, a.len().to_string());
        assert_eq!(d.bytes_read, a.len() as u64);
    }

    #[test]
    fn garbage_and_unknown_types() {
        let d = parse(
            b"{not json\n\n\xff\xfe\n{\"type\":\"zzz\",\"x\":1}\n[1,2]\n{\"type\":\"user\"}\n",
        );
        assert_eq!(d.prompt_count, 0);
        assert_eq!(d.lines, 6);
    }

    #[test]
    fn branch_head_is_none() {
        let d =
            parse(b"{\"type\":\"user\",\"gitBranch\":\"HEAD\",\"message\":{\"content\":\"x\"}}\n");
        assert_eq!(d.branch, None);
    }

    #[test]
    fn artifact_title_from_path() {
        assert_eq!(
            path_title("/a/b/proposal/index.html").as_deref(),
            Some("proposal")
        );
        assert_eq!(
            path_title("/a/b/Catalog plan.html").as_deref(),
            Some("Catalog plan")
        );
        assert_eq!(path_title("/a/Report.html").as_deref(), Some("a"));
        assert_eq!(path_title("index.html").as_deref(), Some("index"));
    }

    #[test]
    fn artifact_title_prefers_input_title() {
        let line = |input: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{{"content":[{{"type":"tool_use","id":"t1","name":"Artifact","input":{input}}}]}}}}
{{"type":"user","timestamp":"2026-01-01T00:00:01Z","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"https://claude.ai/artifact/abc12345-def"}}]}}}}
"#
            )
        };
        let t = |input: &str| parse(line(input).as_bytes()).outputs[0].title.clone();
        assert_eq!(
            t(r#"{"title":"Plan","file_path":"/x/y/index.html"}"#).as_deref(),
            Some("Plan")
        );
        assert_eq!(
            t(r#"{"file_path":"/x/y/index.html"}"#).as_deref(),
            Some("y")
        );
        assert_eq!(
            t(r#"{"url":"https://claude.ai/artifact/abc12345-def"}"#),
            None
        );
        let n = |input: &str| parse(line(input).as_bytes()).outputs.len();
        assert_eq!(n(r#"{"action":"publish","file_path":"/x/a.html"}"#), 1);
        for other in ["quickstart", "read", "list", "open"] {
            assert_eq!(
                n(&format!(r#"{{"action":"{other}","intent":"other"}}"#)),
                0,
                "{other}"
            );
        }
    }
}
