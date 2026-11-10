//! Summaries: build a masked excerpt of a session, ask the summary agent for its status, state and
//! next step, store the answer.

use crate::agents::{
    self, Agent, AgentRef, HeadlessReply, HeadlessRequest, TailItem, PROMPTS_CAP, REPLIES_CAP,
};
use crate::config::Config;
use crate::db::{fts_write, Db};
use crate::model::{AiStatus, Session};
use crate::paths::Paths;
use crate::util::{fmt_ts, head_chars, now_ms};
use anyhow::{Context, Result};
use regex::Regex;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{mpsc, Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

const PROMPT: &str = include_str!("summarize_prompt.txt");

/// The system prompt for a session of the agent whose short name is `agent` ("Claude").
fn system_prompt(agent: &str) -> String {
    PROMPT.replace("{agent}", agent)
}
/// Most bytes of excerpt sent for one session.
pub const EXCERPT_CAP: usize = 48_000;
const TIMEOUT: Duration = Duration::from_secs(120);

static MASKS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let m = |p: &str, r: &'static str| (Regex::new(p).expect("static regex"), r);
    vec![
        m(
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
            "[masked]",
        ),
        m(r"AKIA[0-9A-Z]{16}", "[masked]"),
        m(r"gh[pousr]_[A-Za-z0-9]{36,}", "[masked]"),
        m(r"xox[abpr]-[A-Za-z0-9-]{10,}", "[masked]"),
        m(r"eyJ[\w-]+\.[\w-]+\.[\w-]+", "[masked]"),
        m(r"Bearer\s+[A-Za-z0-9._-]{16,}", "Bearer [masked]"),
        m(
            r#"(?i)(api[_-]?key|secret|token|password|passwd|authorization)(\s*[:=]\s*)("?)[^\s"']{8,}"#,
            "$1$2$3[masked]",
        ),
    ]
});

/// Replace API keys, tokens, passwords and private keys with `[masked]` (best effort).
pub fn mask_secrets(text: &str) -> String {
    let mut t = text.to_string();
    for (re, rep) in MASKS.iter() {
        t = re.replace_all(&t, *rep).into_owned();
    }
    t
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn hash_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// The text sent to the summary agent.
pub struct Excerpt {
    /// What the summary agent is told it does (names the agent that ran the session).
    pub system: String,
    pub text: String,
    /// Hash of everything except the previous summary, so a re-run on unchanged input is a no-op.
    pub hash: String,
}

fn render_items(items: &[TailItem]) -> String {
    let mut out = String::new();
    for it in items {
        match it {
            TailItem::Text { role, text } => out.push_str(&format!("[{role}] {text}\n")),
            TailItem::Tool(l) => out.push_str(&format!("  > {l}\n")),
        }
    }
    out
}

/// The excerpt of session `s`, whose transcript `agent` reads.
pub fn build_excerpt(db: &Db, agent: &dyn Agent, s: &Session) -> Result<Excerpt> {
    let path = s
        .transcript_path
        .as_deref()
        .context("session has no transcript path")?;
    let mut items = agent.conversation_tail(Path::new(path))?;

    let mut head = String::new();
    head.push_str(&format!(
        "# Session: {}\nfolder: {}\nbranch: {}\nkind: {}\nstarted: {}\nlast activity: {}\nprompts: {}\n",
        s.title.as_deref().unwrap_or("(untitled)"),
        s.cwd.as_deref().unwrap_or("?"),
        s.git_branch.as_deref().unwrap_or("-"),
        s.kind.as_deref().unwrap_or("?"),
        s.started_at.map(fmt_ts).unwrap_or_default(),
        s.last_activity_at.map(fmt_ts).unwrap_or_default(),
        s.prompt_count
    ));
    if let Some(r) = &s.recap {
        head.push_str(&format!(
            "\n## {}'s latest recap\n{}\n",
            agent.name(),
            head_chars(r, 3_000)
        ));
    }
    if let Some(p) = &s.first_prompt {
        head.push_str(&format!("\n## First prompt\n{}\n", head_chars(p, 2_000)));
    }
    let outputs = db.outputs_for(&s.id)?;
    if !outputs.is_empty() {
        head.push_str("\n## Outputs\n");
        let mut jira: Vec<&str> = Vec::new();
        let mut shown = 0;
        for o in &outputs {
            match o.kind.as_str() {
                "jira" => jira.push(&o.value),
                "folder" => {}
                _ if shown < 25 => {
                    shown += 1;
                    head.push_str(&format!("- {}: {}\n", o.kind, o.value));
                }
                _ => {}
            }
        }
        if !jira.is_empty() {
            let n = jira.len();
            jira.truncate(30);
            head.push_str(&format!(
                "- jira keys mentioned ({n}): {}\n",
                jira.join(", ")
            ));
        }
    }
    let streams = db.streams_for(&s.id)?;
    let kids: Vec<_> = streams.iter().filter(|x| x.kind == "child").collect();
    if !kids.is_empty() {
        head.push_str("\n## Sub-agents\n");
        for k in kids.iter().take(20) {
            head.push_str(&format!(
                "- {} ({})\n",
                k.title.as_deref().unwrap_or(&k.key),
                k.status.as_deref().unwrap_or("?")
            ));
        }
    }
    let mut prev = String::new();
    if s.state_line.is_some() || s.next_step.is_some() {
        prev.push_str("\n## Previous summary\n");
        prev.push_str(&format!(
            "status: {}\nstate: {}\nnext: {}\n",
            s.ai_status.as_deref().unwrap_or("?"),
            s.state_line.as_deref().unwrap_or(""),
            s.next_step.as_deref().unwrap_or("")
        ));
        for t in streams.iter().filter(|x| x.kind == "topic") {
            prev.push_str(&format!(
                "topic {} | {} | {} | {}\n",
                t.key,
                t.title.as_deref().unwrap_or(""),
                t.status.as_deref().unwrap_or(""),
                t.detail.as_deref().unwrap_or("")
            ));
        }
    }
    let mut head = mask_secrets(&head);
    let prev = mask_secrets(&prev);
    // oldest messages go first when over the cap
    loop {
        let tail = mask_secrets(&render_items(&items));
        let body =
            format!("{head}\n## Last messages (oldest first, the last one is the newest)\n{tail}");
        if body.len() + prev.len() <= EXCERPT_CAP || items.is_empty() {
            let mut text = format!("{body}{prev}");
            if text.len() > EXCERPT_CAP {
                text = head_chars(&text, EXCERPT_CAP);
            }
            let system = system_prompt(agent.short_name());
            return Ok(Excerpt {
                hash: hash_hex(&format!("{system}\n{body}")),
                system,
                text,
            });
        }
        items.remove(0);
        if head.len() > EXCERPT_CAP / 2 {
            head = head_chars(&head, EXCERPT_CAP / 2);
        }
    }
}

/// The JSON schema the summary must follow.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "status": {"type": "string", "enum": ["done", "paused", "waiting_on_others", "needs_you"]},
            "state": {"type": "string", "maxLength": 140},
            "next": {"type": "string", "maxLength": 140},
            "topics": {"type": "array", "maxItems": 5, "items": {
                "type": "object",
                "properties": {
                    "key": {"type": "string"},
                    "title": {"type": "string", "maxLength": 40},
                    "status": {"type": "string", "enum": ["done", "active", "paused", "waiting_on_others", "needs_you"]},
                    "next": {"type": "string", "maxLength": 100}
                },
                "required": ["key", "title", "status", "next"]
            }}
        },
        "required": ["status", "state", "next", "topics"]
    })
}

/// A topic of a session: a separate thing it worked on.
#[derive(Debug, Clone, PartialEq)]
pub struct Topic {
    pub key: String,
    pub title: String,
    pub status: String,
    pub next: String,
}

/// A summary as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub status: AiStatus,
    pub status_str: String,
    pub state: String,
    pub next: String,
    pub topics: Vec<Topic>,
}

/// A summary with what the call cost.
#[derive(Debug, Clone)]
pub struct CallResult {
    pub summary: Summary,
    pub duration_ms: i64,
    pub usage: Value,
    pub cost_usd: Option<f64>,
}

/// Read the structured answer of the summary agent.
pub fn call_result(reply: HeadlessReply) -> Result<CallResult> {
    let so = &reply.output;
    let str_of = |k: &str, max: usize| {
        head_chars(so.get(k).and_then(Value::as_str).unwrap_or("").trim(), max)
    };
    let status_str = str_of("status", 40);
    let status =
        AiStatus::parse(&status_str).with_context(|| format!("bad status '{status_str}'"))?;
    let mut topics = Vec::new();
    if let Some(a) = so.get("topics").and_then(Value::as_array) {
        for t in a.iter().take(5) {
            let g = |k: &str, max: usize| {
                head_chars(t.get(k).and_then(Value::as_str).unwrap_or("").trim(), max)
            };
            let key = g("key", 60);
            if key.is_empty() {
                continue;
            }
            topics.push(Topic {
                key,
                title: g("title", 40),
                status: g("status", 24),
                next: g("next", 100),
            });
        }
    }
    Ok(CallResult {
        summary: Summary {
            status,
            status_str,
            state: str_of("state", 140),
            next: if status == AiStatus::Done {
                String::new()
            } else {
                str_of("next", 140)
            },
            topics,
        },
        duration_ms: reply.duration_ms,
        usage: reply.usage,
        cost_usd: reply.cost_usd,
    })
}

/// Why a session is not summarized, if it is not.
pub fn skip_reason(cfg: &Config, s: &Session) -> Option<&'static str> {
    if s.kind.as_deref() == Some("history_only") {
        Some("history only")
    } else if s.prompt_count == 0 {
        Some("no prompts")
    } else if s.archived {
        Some("archived")
    } else if s.cwd.as_deref().is_some_and(|c| cfg.is_excluded(c)) {
        Some("excluded folder")
    } else if s.transcript_state.as_deref() != Some("present") {
        Some("transcript not present")
    } else {
        None
    }
}

/// Store a summary and its topics.
pub fn store(db: &Db, s: &Session, hash: &str, r: &CallResult, extra: Value) -> Result<()> {
    let now = now_ms();
    let sm = &r.summary;
    let tx = db.conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE session SET ai_status=?2, state_line=?3, next_step=?4, summary_at=?5, summary_input_hash=?6 WHERE id=?1",
        params![s.id, sm.status_str, sm.state, sm.next, now, hash],
    )?;
    let keys: Vec<&str> = sm.topics.iter().map(|t| t.key.as_str()).collect();
    let old: Vec<String> = {
        let mut st = tx.prepare("SELECT key FROM stream WHERE session_id=?1 AND kind='topic'")?;
        let rows = st.query_map([&s.id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for k in old.iter().filter(|k| !keys.contains(&k.as_str())) {
        tx.execute(
            "DELETE FROM stream WHERE session_id=?1 AND kind='topic' AND key=?2",
            params![s.id, k],
        )?;
    }
    for (i, t) in sm.topics.iter().enumerate() {
        tx.execute(
            "INSERT INTO stream(session_id, kind, key, title, status, detail, sort, updated_at) VALUES(?1,'topic',?2,?3,?4,?5,?6,?7)
             ON CONFLICT(session_id, kind, key) DO UPDATE SET title=excluded.title, status=excluded.status,
               detail=excluded.detail, sort=excluded.sort, updated_at=excluded.updated_at",
            params![s.id, t.key, t.title, t.status, t.next, i as i64, now],
        )?;
    }
    let mut payload = json!({
        "duration_ms": r.duration_ms,
        "usage": r.usage,
        "total_cost_usd": r.cost_usd,
        "status": sm.status_str,
    });
    if let (Some(p), Some(e)) = (payload.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    tx.execute(
        "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,'summary',?3)",
        params![s.id, now, payload.to_string()],
    )?;
    fts_write(&tx, &s.id, "", "", false, PROMPTS_CAP, REPLIES_CAP)?;
    tx.commit()?;
    Ok(())
}

/// The model the summaries ask for: `[summary] model`, else the default of the agent that writes them.
pub fn summary_model(cfg: &Config, agent: &dyn Agent) -> String {
    match cfg.summary.model.trim() {
        "" => agent.default_summary_model().to_string(),
        m => m.to_string(),
    }
}

/// What `summarize` should do.
#[derive(Debug, Default)]
pub struct Options {
    pub ids: Vec<String>,
    pub all: bool,
    pub dry_run: bool,
    pub max: Option<usize>,
    pub force: bool,
    pub print_excerpt: bool,
}

/// What happened to one session.
#[derive(Debug, serde::Serialize)]
pub struct Outcome {
    pub id: String,
    pub title: Option<String>,
    pub result: String,
    pub excerpt_bytes: usize,
    pub wall_ms: i64,
    /// Absent when no call was made or the agent reports no cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub summary: Option<Value>,
}

struct Job {
    session: Session,
    excerpt: Excerpt,
}

/// Summarize the sessions in `opts`, or the quiet ones it is told to.
pub fn summarize(db: &Db, paths: &Paths, cfg: &Config, opts: &Options) -> Result<Vec<Outcome>> {
    let mut outcomes = Vec::new();
    if !cfg.summary.enabled {
        println!("summaries are disabled (summary.enabled = false)");
        return Ok(outcomes);
    }
    let mut sessions: Vec<Session> = Vec::new();
    for id in &opts.ids {
        let id = db.resolve_id(id)?;
        sessions.extend(db.session_by_id(&id)?);
    }
    if opts.all {
        let mut all = db.all_sessions()?;
        all.sort_by_key(|s| std::cmp::Reverse(s.last_activity_at.unwrap_or(0)));
        sessions.extend(all);
    }
    let agents = agents::enabled(cfg);
    let Some(ask_agent) = agents.summary_agent(cfg).cloned() else {
        println!("no agent is enabled to write summaries (see [agents] and [summary] agent)");
        return Ok(outcomes);
    };
    let hidden = crate::hide::hidden_ids(db, cfg)?;
    let mut jobs: VecDeque<Job> = VecDeque::new();
    for s in sessions {
        if opts.max.is_some_and(|m| jobs.len() >= m) {
            break;
        }
        let mk = |result: &str, bytes: usize| Outcome {
            id: s.id.clone(),
            title: s.title.clone(),
            result: result.into(),
            excerpt_bytes: bytes,
            wall_ms: 0,
            cost_usd: None,
            summary: None,
        };
        if hidden.contains(&s.id) {
            outcomes.push(mk("skipped: hidden", 0));
            continue;
        }
        if let Some(why) = skip_reason(cfg, &s) {
            outcomes.push(mk(&format!("skipped: {why}"), 0));
            continue;
        }
        let Some(session_agent) = agents.of_session(&s) else {
            outcomes.push(mk("skipped: its agent is not enabled", 0));
            continue;
        };
        let ex = match build_excerpt(db, session_agent.as_ref(), &s) {
            Ok(e) => e,
            Err(e) => {
                outcomes.push(mk(&format!("skipped: {e:#}"), 0));
                continue;
            }
        };
        if !opts.force && s.summary_input_hash.as_deref() == Some(ex.hash.as_str()) {
            outcomes.push(mk("skipped: input unchanged", ex.text.len()));
            continue;
        }
        if opts.print_excerpt {
            println!("{}\n=====", ex.text);
        }
        if opts.dry_run {
            outcomes.push(mk("dry-run: would summarize", ex.text.len()));
            continue;
        }
        jobs.push_back(Job {
            session: s,
            excerpt: ex,
        });
    }
    if jobs.is_empty() {
        return Ok(outcomes);
    }
    let work = paths.home.join("work");
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let queue = Arc::new(Mutex::new(jobs));
    let (tx, rx) = mpsc::channel::<(Job, Result<(CallResult, i64)>)>();
    let workers = cfg.summary.concurrency.max(1) as usize;
    let model = summary_model(cfg, ask_agent.as_ref());
    let mut handles = Vec::new();
    for _ in 0..workers {
        let (queue, tx, work, model) = (queue.clone(), tx.clone(), work.clone(), model.clone());
        let ask_agent: AgentRef = ask_agent.clone();
        handles.push(std::thread::spawn(move || loop {
            let job = match queue.lock() {
                Ok(mut q) => q.pop_front(),
                Err(_) => None,
            };
            let Some(job) = job else { break };
            let t0 = Instant::now();
            let res = ask_agent
                .ask(&HeadlessRequest {
                    model: &model,
                    system: &job.excerpt.system,
                    input: &job.excerpt.text,
                    schema: &schema(),
                    work_dir: &work,
                    timeout: TIMEOUT,
                })
                .and_then(call_result)
                .map(|r| (r, t0.elapsed().as_millis() as i64));
            if tx.send((job, res)).is_err() {
                break;
            }
        }));
    }
    drop(tx);
    for (job, res) in rx {
        let s = &job.session;
        let bytes = job.excerpt.text.len();
        match res {
            Ok((r, wall)) => {
                store(
                    db,
                    s,
                    &job.excerpt.hash,
                    &r,
                    json!({"wall_ms": wall, "excerpt_bytes": bytes, "model": model}),
                )?;
                let sm = &r.summary;
                outcomes.push(Outcome {
                    id: s.id.clone(),
                    title: s.title.clone(),
                    result: "summarized".into(),
                    excerpt_bytes: bytes,
                    wall_ms: wall,
                    cost_usd: r.cost_usd,
                    summary: Some(json!({
                        "status": sm.status_str, "state": sm.state, "next": sm.next,
                        "topics": sm.topics.iter().map(|t| json!({"key": t.key, "title": t.title, "status": t.status, "next": t.next})).collect::<Vec<_>>(),
                    })),
                });
            }
            Err(e) => outcomes.push(Outcome {
                id: s.id.clone(),
                title: s.title.clone(),
                result: format!("failed: {e:#}"),
                excerpt_bytes: bytes,
                wall_ms: 0,
                cost_usd: None,
                summary: None,
            }),
        }
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use std::io::Write;
    use std::path::Path;

    fn claude() -> Claude {
        Claude::new("/nonexistent/c".into())
    }

    #[test]
    fn the_model_is_the_one_configured_else_the_default_of_the_agent_that_writes() {
        let mut cfg = Config::default();
        assert_eq!(summary_model(&cfg, &claude()), "haiku");
        assert_eq!(
            summary_model(&cfg, &crate::agents::codex::Codex::new("/x".into())),
            "gpt-6-luna"
        );
        cfg.summary.model = " opus ".into();
        assert_eq!(summary_model(&cfg, &claude()), "opus");
    }

    #[test]
    fn masks_secrets() {
        // made-up secrets, assembled at run time so no token-shaped literal sits in the code
        let gh = format!("{}_{}", "ghp", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ12");
        let t = &format!(
            "key AKIAABCDEFGHIJKLMNOP and {gh} and xoxb-1234567890-abc \
                 jwt eyJhbGciOi.eyJzdWIiOi.SflKxwRJSM Authorization: Bearer abcdefghijklmnop12345 \
                 password=hunter2hunter2 api_key: \"sk12345678\" short token=abc\n\
                 -----BEGIN RSA PRIVATE KEY-----\nMIIE\nabc\n-----END RSA PRIVATE KEY-----\nafter"
        );
        let m = mask_secrets(t);
        for bad in [
            "AKIAABCDEFGHIJKLMNOP",
            "ghp_abc",
            "xoxb-1234",
            "eyJhbGci",
            "abcdefghijklmnop12345",
            "hunter2hunter2",
            "sk12345678",
            "MIIE",
        ] {
            assert!(!m.contains(bad), "{bad} leaked in {m}");
        }
        assert!(m.contains("password=[masked]"));
        assert!(m.contains("token=abc"));
        assert!(m.contains("after"));
    }

    fn fixture_session(path: &Path) -> Session {
        Session {
            id: "s1".into(),
            title: Some("Demo".into()),
            cwd: Some("/tmp/demo".into()),
            kind: Some("interactive".into()),
            prompt_count: 3,
            first_prompt: Some("start".into()),
            transcript_path: Some(path.to_string_lossy().into()),
            transcript_state: Some("present".into()),
            ..Default::default()
        }
    }

    fn line(role: &str, content: Value) -> String {
        format!(
            "{}\n",
            json!({"message": {"content": content}, "type": role, "timestamp": "2026-11-01T10:00:00Z"})
        )
    }

    #[test]
    fn excerpt_masks_and_skips_tool_results() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        let w = |f: &mut std::fs::File, s: String| f.write_all(s.as_bytes()).unwrap();
        w(
            &mut f,
            "{\"type\":\"attachment\",\"x\":\"MEMORY\"}\n".into(),
        );
        w(&mut f, "garbage line\n".into());
        w(
            &mut f,
            line("user", json!("deploy it, token=supersecret123")),
        );
        w(
            &mut f,
            line(
                "assistant",
                json!([
                    {"type":"text","text":"Running the build"},
                    {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"make build\nmore"}},
                    {"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/x"}},
                    {"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/src/a.rs"}}
                ]),
            ),
        );
        w(
            &mut f,
            line(
                "user",
                json!([
                    {"type":"tool_result","tool_use_id":"t1","content":"SECRET-BODY-OUTPUT"},
                    {"type":"tool_result","tool_use_id":"t3","is_error":true,"content":"boom failed"}
                ]),
            ),
        );
        let db = Db::open_memory().unwrap();
        let ex = build_excerpt(&db, &claude(), &fixture_session(&p)).unwrap();
        assert!(ex.text.contains("token=[masked]"));
        assert!(!ex.text.contains("supersecret"));
        assert!(!ex.text.contains("SECRET-BODY-OUTPUT"));
        assert!(!ex.text.contains("MEMORY"));
        assert!(ex.text.contains("> Bash: make build more"));
        assert!(ex.text.contains("> Edit: /src/a.rs"));
        assert!(!ex.text.contains("Read:"));
        assert!(ex.text.contains("error: boom failed"));
        assert!(ex.text.contains("[assistant] Running the build"));
    }

    #[test]
    fn excerpt_size_cap_and_message_limit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        for i in 0..100 {
            let big = format!("{} m{i}", "x".repeat(5_000));
            f.write_all(line("user", json!(big)).as_bytes()).unwrap();
        }
        let db = Db::open_memory().unwrap();
        let ex = build_excerpt(&db, &claude(), &fixture_session(&p)).unwrap();
        assert!(ex.text.len() <= EXCERPT_CAP, "{}", ex.text.len());
        assert!(ex.text.contains("m99\n"));
        assert!(!ex.text.contains("m60\n"));
        // each message keeps at most 1.5 KB, from the end
        assert!(!ex.text.contains(&"x".repeat(1_600)));
    }

    #[test]
    fn parses_recorded_result_and_stores() {
        let reply = |output: Value| HeadlessReply {
            output,
            duration_ms: 3210,
            usage: json!({"input_tokens": 10, "output_tokens": 5}),
            cost_usd: Some(0.0123),
        };
        let r = call_result(reply(json!({
            "status": "done", "state": "PR merged", "next": "ignored when done",
            "topics": [
                {"key": "pr", "title": "The PR", "status": "done", "next": ""},
                {"key": "", "title": "x", "status": "done", "next": ""}
            ]
        })))
        .unwrap();
        assert_eq!(r.summary.status, AiStatus::Done);
        assert_eq!(r.summary.next, "");
        assert_eq!(r.summary.topics.len(), 1);
        assert_eq!(r.duration_ms, 3210);
        assert!(call_result(reply(json!({"status": "bogus"}))).is_err());

        let db = Db::open_memory().unwrap();
        db.conn
            .execute("INSERT INTO session(id,title) VALUES('s1','T')", [])
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO stream(session_id,kind,key,title) VALUES('s1','topic','old','Old')",
                [],
            )
            .unwrap();
        let s = db.session_by_id("s1").unwrap().unwrap();
        store(&db, &s, "h", &r, json!({"wall_ms": 5})).unwrap();
        let s = db.session_by_id("s1").unwrap().unwrap();
        assert_eq!(s.ai_status.as_deref(), Some("done"));
        assert_eq!(s.state_line.as_deref(), Some("PR merged"));
        assert_eq!(s.summary_input_hash.as_deref(), Some("h"));
        let topics: Vec<String> = db
            .streams_for("s1")
            .unwrap()
            .into_iter()
            .filter(|x| x.kind == "topic")
            .map(|x| x.key)
            .collect();
        assert_eq!(topics, vec!["pr"]);
        let payload: String = db
            .conn
            .query_row("SELECT payload FROM event WHERE kind='summary'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(payload.contains("0.0123") && payload.contains("output_tokens"));
    }

    #[test]
    fn the_prompt_names_the_agent_and_not_the_user() {
        let p = system_prompt("Claude");
        assert!(p.contains("\"Claude\" for the agent") && p.contains("\"You\" for the user"));
        assert!(!p.contains("{agent}"));
        // the excerpt and its hash follow the agent's name
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(&path, line("user", json!("hello"))).unwrap();
        let db = Db::open_memory().unwrap();
        let ex = build_excerpt(&db, &claude(), &fixture_session(&path)).unwrap();
        assert!(ex.system.contains("Claude session"));
    }

    #[test]
    fn skip_rules() {
        let cfg = Config::default();
        let mut s = fixture_session(Path::new("/x"));
        assert_eq!(skip_reason(&cfg, &s), None);
        s.prompt_count = 0;
        assert_eq!(skip_reason(&cfg, &s), Some("no prompts"));
        s.prompt_count = 2;
        s.archived = true;
        assert_eq!(skip_reason(&cfg, &s), Some("archived"));
        s.archived = false;
        s.kind = Some("history_only".into());
        assert!(skip_reason(&cfg, &s).is_some());
    }
}
