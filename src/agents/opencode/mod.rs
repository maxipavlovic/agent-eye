//! OpenCode (`opencode`), checked on a real OpenCode 2.3.2. Sessions are rows of a SQLite database
//! (`session_v2`, or the older `session` tables) or, before v1.2, JSON files; see `store.rs`. A
//! database session has no file of its own, so its transcript path is `<db>#<session id>` and the
//! adapter says whether it exists (`Agent::transcript_exists`). eye reads a session whole whenever
//! its row count or update time moves.
//!
//! - live status: `live.rs` reads the turn times OpenCode writes on the session's row; a permission
//!   prompt is seen by the plugin eye installs (`plugin.js`, `hooks.rs`);
//! - summaries and judging: `headless.rs`, `opencode run --format json` with a JSON-only prompt,
//!   validated, locked down as every `opencode` command eye runs is (`lockdown.rs`);
//! - archive and restore: the archive keeps `opencode session export`, restore is `session import`
//!   (`cli.rs`); eye never writes OpenCode's database;
//! - install: the plugin and the `/eye` command (`install.rs`).

mod cli;
mod events;
mod headless;
mod hooks;
mod install;
mod live;
mod lockdown;
#[cfg(test)]
mod run_tests;
mod store;
mod turns;

use super::*;
use crate::config::AgentCfg;
use crate::report::Report;
use crate::util::{find_in_path, head_chars, tail_chars, trim_front_bytes};
use anyhow::{Context, Result};
use events::{Ev, Loaded};
use serde_json::Value;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The agent id.
pub const ID: &str = "opencode";
/// The short name used in sentences and `next` lines.
pub const SHORT: &str = "OpenCode";
const PROGRAM: &str = "opencode";
const MSG_CAP: usize = 4_000;

/// The OpenCode adapter.
#[derive(Debug)]
pub struct OpenCode {
    dir: PathBuf,
    /// OpenCode's config folder: where its plugins and commands are.
    config_dir: PathBuf,
    exe: Option<String>,
}

/// The factory the registry uses.
pub fn build(cfg: &AgentCfg) -> AgentRef {
    Arc::new(OpenCode::new(resolve_dir(cfg)))
}

/// `[agents.opencode] dir`, else `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode` (the
/// only variable OpenCode's code reads for it).
fn resolve_dir(cfg: &AgentCfg) -> PathBuf {
    cfg.dir_override
        .clone()
        .or_else(|| cfg.dir.as_deref().map(crate::config::expand_tilde))
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .filter(|v| !v.is_empty())
                .map(|v| PathBuf::from(v).join("opencode"))
        })
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/nonexistent"))
                .join(".local/share/opencode")
        })
}

/// `$XDG_CONFIG_HOME/opencode`, else `~/.config/opencode`.
fn default_config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(|v| PathBuf::from(v).join("opencode"))
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/nonexistent"))
                .join(".config/opencode")
        })
}

impl OpenCode {
    /// An adapter for OpenCode's data folder `dir`.
    pub fn new(dir: PathBuf) -> OpenCode {
        OpenCode {
            dir,
            config_dir: default_config_dir(),
            exe: None,
        }
    }

    /// Install the plugin and the command in this config folder instead of the usual one.
    pub fn with_config_dir(mut self, dir: PathBuf) -> OpenCode {
        self.config_dir = dir;
        self
    }

    /// Run this program instead of `opencode`.
    pub fn with_executable(mut self, exe: &str) -> OpenCode {
        self.exe = Some(exe.to_string());
        self
    }

    fn executable_path(&self) -> String {
        self.exe
            .clone()
            .or_else(|| {
                std::env::var("EYE_OPENCODE_BIN")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| PROGRAM.into())
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

fn delta_of(l: &Loaded, session_id: &str) -> Delta {
    use super::outputs::{scan_jira, scan_jira_urls, scan_pr, Seen};
    let mut d = Delta::default();
    let mut seen = Seen::new();
    for e in &l.events {
        let ts = e.ts();
        if ts > 0 {
            d.first_ts = Some(d.first_ts.map_or(ts, |t| t.min(ts)));
            d.last_ts = Some(d.last_ts.map_or(ts, |t| t.max(ts)));
            d.events.push(WorkEvent {
                ts,
                prompt: matches!(e, Ev::Prompt { .. }),
            });
        }
        match e {
            Ev::Prompt { text, .. } => {
                d.prompt_count += 1;
                for pre in scan_jira(&mut d, &mut seen, text, ts) {
                    d.jira_prompt_prefixes.insert(pre);
                }
                let p = head_chars(text, MSG_CAP);
                d.first_prompt.get_or_insert_with(|| head_chars(&p, 2_000));
                d.last_user_text = Some(head_chars(&p, 1_500));
                d.prompts_text.push_str(&p);
                d.prompts_text.push('\n');
            }
            Ev::Reply { text, .. } => {
                d.reply_count += 1;
                scan_jira(&mut d, &mut seen, text, ts);
                scan_pr(&mut d, &mut seen, text, ts);
                scan_jira_urls(&mut d, text);
                d.last_assistant_text = Some(tail_chars(text, 1_500));
                d.replies_text.push_str(&head_chars(text, MSG_CAP));
                d.replies_text.push('\n');
            }
            Ev::Tool { .. } => {}
            Ev::Usage {
                ts,
                model,
                input,
                output,
                cache_read,
                cache_write,
            } => {
                d.usage.input += input;
                d.usage.output += output;
                d.usage.cache_read += cache_read;
                d.usage.cache_write += cache_write;
                d.usage_msgs.push(UsageMsg {
                    ts: *ts,
                    model: model.clone(),
                    input: *input,
                    output: *output,
                    cache_write: *cache_write,
                    cache_read: *cache_read,
                });
            }
        }
    }
    trim_front_bytes(&mut d.prompts_text, PROMPTS_CAP);
    trim_front_bytes(&mut d.replies_text, REPLIES_CAP);
    d.turns = l.turns.clone();
    d.lines = l.events.len() as u64;
    d.session_id = Some(session_id.to_string());
    d.cwd = Some(l.directory.clone()).filter(|c| !c.is_empty());
    d.ai_title = l.title.clone().filter(|t| !t.trim().is_empty());
    d.replaces = true;
    d
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

impl Agent for OpenCode {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "OpenCode"
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
                (false, None) => format!("no {} and no `{PROGRAM}` on PATH", self.dir.display()),
            },
        }
    }

    fn transcripts(&self) -> Vec<Transcript> {
        store::scan(&self.dir)
    }

    /// A session is read whole each time its stamp (message count, update time) moves.
    fn parse_transcript(&self, t: &Transcript, _opts: &ParseOpts) -> Result<Delta> {
        let l = store::load(&t.path)?;
        let mut d = delta_of(&l, &t.session_id);
        d.marker = t.stamp.size.to_string();
        d.bytes_read = l.events.len() as u64;
        Ok(d)
    }

    fn work_events(&self, path: &Path) -> Result<Vec<WorkEvent>> {
        Ok(store::events(path)?
            .iter()
            .filter(|e| e.ts() > 0)
            .map(|e| WorkEvent {
                ts: e.ts(),
                prompt: matches!(e, Ev::Prompt { .. }),
            })
            .collect())
    }

    fn turn_events(&self, path: &Path) -> Result<Vec<TurnEvent>> {
        Ok(store::load(path)?.turns)
    }

    fn turn_kinds(&self) -> &'static [TurnKind] {
        crate::agents::ALL_TURN_KINDS
    }

    /// Not proven: the deny rules are built from OpenCode's documentation and have not been checked
    /// against a real run (`docs/security.md`).
    fn judging_locked_down(&self) -> bool {
        false
    }

    /// What a `think` event counts: the reasoning tokens of an assistant message.
    fn think_unit(&self) -> Option<&'static str> {
        Some("reasoning_tokens")
    }

    fn usage_msgs(&self, path: &Path) -> Result<Vec<UsageMsg>> {
        Ok(delta_of(&store::load(path)?, "").usage_msgs)
    }

    /// A session of this adapter's database, or a file of its older layout.
    fn owns_transcript(&self, path: &Path) -> bool {
        match store::split_db_path(path) {
            Some((db, _)) => db.parent() == Some(self.dir.as_path()),
            None => crate::agents::is_inside(&self.dir, path),
        }
    }

    fn transcript_exists(&self, path: &Path) -> bool {
        store::exists(path)
    }

    /// The session's rows (and its sub-agents' rows) in the database, or its JSON files.
    fn delete_transcript(&self, path: &Path, session_id: &str) -> Result<Option<Vec<String>>> {
        store::delete(&self.dir, path, session_id).map(Some)
    }

    fn own_dirs(&self) -> Vec<String> {
        vec![self.dir.to_string_lossy().into_owned()]
    }

    fn transcript_stamp(&self, path: &Path) -> Option<Stamp> {
        store::stamp(path)
    }

    fn archive_name(&self, session_id: &str) -> String {
        format!("{session_id}.json.zst")
    }

    /// The session as one JSON export, compressed: what eye reads, and `opencode session export`
    /// when OpenCode can be asked (what restoring needs).
    fn archive(&self, path: &Path, copy: &Path) -> Result<()> {
        let mut l = store::load(path)?;
        let id = match store::split_db_path(path) {
            Some((db, id)) => db.is_file().then_some(id),
            None => path.file_stem().and_then(|s| s.to_str()).map(String::from),
        };
        if let Some(id) = id {
            l.native = self.export_native(&id);
        }
        let bytes = zstd::encode_all(serde_json::to_vec(&l)?.as_slice(), 3)?;
        write_private(copy, &bytes)
    }

    /// Put the session back into OpenCode with `opencode session import`, so that `opencode
    /// --session <id>` resumes it after OpenCode deleted it. A copy made without OpenCode's export
    /// can only be read.
    fn restore_archived(&self, copy: &Path, dest: &Path) -> Option<Result<()>> {
        Some((|| {
            anyhow::ensure!(
                !self.has_session(dest),
                "the session is still in OpenCode; nothing restored"
            );
            let json = zstd::decode_all(std::fs::File::open(copy)?).context("decompressing")?;
            let l: events::Loaded =
                serde_json::from_slice(&json).context("reading the archived copy")?;
            let native = l.native.context(
                "this copy was made without `opencode session export` (OpenCode was not there), so it can be read but not put back",
            )?;
            let tmp = copy.with_extension("import.json");
            self.import_native(&native, &tmp)?;
            anyhow::ensure!(
                self.has_session(dest),
                "`opencode session import` ran but the session is not in {}",
                crate::util::show_path(&self.dir)
            );
            Ok(())
        })())
    }

    fn unarchive(&self, copy: &Path, dest: &Path) -> Result<()> {
        let json = zstd::decode_all(std::fs::File::open(copy)?).context("decompressing")?;
        write_private(dest, &json)
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<crate::conversation::Message>> {
        let mut out: Vec<crate::conversation::Message> = vec![];
        let (mut texts, mut tools, mut ts): (Vec<String>, u32, Option<i64>) = (vec![], 0, None);
        let flush = |out: &mut Vec<crate::conversation::Message>,
                     texts: &mut Vec<String>,
                     tools: &mut u32,
                     ts: &mut Option<i64>| {
            if !texts.is_empty() || *tools > 0 {
                out.push(crate::conversation::Message::new(
                    "assistant",
                    &texts.join("\n\n"),
                    *ts,
                    *tools,
                ));
            }
            *texts = vec![];
            *tools = 0;
            *ts = None;
        };
        for e in store::events(path)? {
            match e {
                Ev::Prompt { ts: pts, text } => {
                    flush(&mut out, &mut texts, &mut tools, &mut ts);
                    out.push(crate::conversation::Message::new(
                        "user",
                        &text,
                        Some(pts),
                        0,
                    ));
                }
                Ev::Reply { ts: t, text } => {
                    texts.push(text);
                    ts = Some(t);
                }
                Ev::Tool { ts: t, .. } => {
                    tools += 1;
                    ts = Some(t);
                }
                Ev::Usage { .. } => {}
            }
        }
        flush(&mut out, &mut texts, &mut tools, &mut ts);
        Ok(out)
    }

    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        let mut items: std::collections::VecDeque<TailItem> = Default::default();
        let mut texts = 0usize;
        for e in store::events(path)? {
            match e {
                Ev::Prompt { text, .. } => {
                    texts += 1;
                    items.push_back(TailItem::Text {
                        role: "user",
                        text: tail_chars(&text, 1_500),
                    });
                }
                Ev::Reply { text, .. } => {
                    texts += 1;
                    items.push_back(TailItem::Text {
                        role: "assistant",
                        text: tail_chars(&text, 1_500),
                    });
                }
                Ev::Tool { line, .. } => items.push_back(TailItem::Tool(line)),
                Ev::Usage { .. } => {}
            }
            while texts > 30 || items.len() > 90 {
                if let Some(TailItem::Text { .. }) = items.pop_front() {
                    texts -= 1;
                }
            }
        }
        Ok(items.into())
    }

    /// A session is live while its turn runs (or just ended); the plugin's prompts win over `busy`.
    fn live_sessions(&self) -> Vec<LiveSession> {
        live::sessions(&self.dir)
    }

    /// The turn times on the session's row do not show a permission prompt; the plugin does.
    fn live_status_shows_prompts(&self) -> bool {
        false
    }

    fn queue_line(&self, event: &str, payload: &Value, ts: i64) -> Option<Value> {
        hooks::queue_line(event, payload, ts)
    }

    fn wait_effect(&self, kind: &str, event: &Value, ts: i64) -> Option<Wait> {
        hooks::wait_effect(kind, event, ts)
    }

    fn ask(&self, req: &HeadlessRequest) -> Result<HeadlessReply> {
        headless::ask(self, req)
    }

    /// `free`: the small free model `opencode models` lists (names of free models come and go).
    fn default_summary_model(&self) -> &'static str {
        headless::FREE
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

    fn daemon_env(&self) -> Vec<(String, String)> {
        self.env()
            .into_iter()
            .filter(|(k, v)| {
                k == "XDG_DATA_HOME" && std::env::var(k).ok().as_deref() != Some(v.as_str())
            })
            .collect()
    }

    fn executable(&self) -> String {
        self.executable_path()
    }

    /// `opencode --session <id>`: the terminal UI on that session.
    fn resume(&self, session_id: &str) -> Invocation {
        Invocation::new(PROGRAM, &["--session", session_id])
    }

    /// `opencode --prompt <text>` in the idea's folder; OpenCode picks the session id itself.
    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        anyhow::ensure!(
            !req.background,
            "OpenCode cannot run a session in the background: start the idea in the foreground"
        );
        let args = match req.prompt {
            Some(p) => vec!["--prompt".to_string(), p.to_string()],
            None => vec![],
        };
        Ok(StartPlan {
            invocation: Invocation {
                program: PROGRAM.into(),
                args,
            },
            session_id: None,
        })
    }

    fn executable_dir(&self) -> Option<PathBuf> {
        self.on_path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    }
}
