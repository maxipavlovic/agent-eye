//! A tiny second agent with its own transcript format, used to prove that the agent layer needs
//! no change in the core: it is registered through the same trait as Claude Code.
//!
//! Its files: `<dir>/logs/<project>/<session>.log`, one event per line, `u|<ms>|<text>` for a user
//! prompt, `a|<ms>|<text>` for a reply and `r|<ms>|<text>` for a recap. The first line is
//! `cwd|<folder>`. A prompt that starts with `!` is one of its built-in commands, and every recap
//! ends with a footer.

use agent_eye::agents::{
    self, append_only, Agent, AgentRef, Delta, Detection, Invocation, ParseOpts, Stamp, StartPlan,
    StartRequest, TailItem, Transcript, PROMPTS_CAP, REPLIES_CAP,
};
use agent_eye::config::AgentCfg;
use agent_eye::conversation::Message;
use anyhow::Result;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const ID: &str = "fake";
/// The tail the fake agent adds to every recap.
pub const RECAP_FOOTER: &str = " [fake footer]";

#[derive(Debug)]
pub struct Fake {
    dir: PathBuf,
}

/// Put the fake agent in the registry, so a config can enable it.
pub fn register() {
    agents::register(ID, |cfg: &AgentCfg| -> AgentRef {
        Arc::new(Fake {
            dir: PathBuf::from(cfg.dir.clone().unwrap_or_default()),
        })
    });
}

impl Agent for Fake {
    fn id(&self) -> &'static str {
        ID
    }
    fn name(&self) -> &'static str {
        "Fake Agent"
    }
    fn short_name(&self) -> &'static str {
        "Fakebot"
    }
    fn detect(&self) -> Detection {
        Detection {
            installed: self.dir.is_dir(),
            detail: format!("looked in {}", self.dir.display()),
        }
    }

    fn transcripts(&self) -> Vec<Transcript> {
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(self.dir.join("logs")) else {
            return out;
        };
        for project in rd.flatten().filter(|p| p.path().is_dir()) {
            for f in std::fs::read_dir(project.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let p = f.path();
                let (Some(stem), Ok(md)) = (p.file_stem().and_then(|s| s.to_str()), f.metadata())
                else {
                    continue;
                };
                out.push(Transcript {
                    project_dir: project.file_name().to_string_lossy().into_owned(),
                    session_id: stem.to_string(),
                    path: p.clone(),
                    stamp: Stamp::of_metadata(&md),
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    fn parse_transcript(&self, file: &Transcript, opts: &ParseOpts) -> Result<Delta> {
        // the file only grows: go on from the byte offset kept as the marker
        let from = append_only::resume_offset(opts.since, file.stamp);
        let mut f = std::fs::File::open(&file.path)?;
        f.seek(SeekFrom::Start(from.unwrap_or(0)))?;
        let mut r = BufReader::new(f);
        let mut offset = from.unwrap_or(0);
        let mut d = Delta {
            replaces: from.is_none(),
            session_id: Some(file.session_id.clone()),
            ..Delta::default()
        };
        let mut line = String::new();
        loop {
            line.clear();
            if r.read_line(&mut line)? == 0 || !line.ends_with('\n') {
                break;
            }
            offset += line.len() as u64;
            d.lines += 1;
            let parts: Vec<&str> = line.trim_end().splitn(3, '|').collect();
            match parts.as_slice() {
                ["cwd", cwd] => d.cwd = Some(cwd.to_string()),
                ["r", ts, text] => d.recap = Some((ts.parse().unwrap_or(0), text.to_string())),
                [kind @ ("u" | "a"), ts, text] => {
                    let ts: i64 = ts.parse().unwrap_or(0);
                    d.first_ts = Some(d.first_ts.map_or(ts, |t| t.min(ts)));
                    d.last_ts = Some(d.last_ts.map_or(ts, |t| t.max(ts)));
                    if *kind == "u" {
                        d.prompt_count += 1;
                        d.first_prompt.get_or_insert_with(|| text.to_string());
                        d.last_user_text = Some(text.to_string());
                        d.prompts_text.push_str(text);
                        d.prompts_text.push('\n');
                    } else {
                        d.reply_count += 1;
                        d.last_assistant_text = Some(text.to_string());
                        d.replies_text.push_str(text);
                        d.replies_text.push('\n');
                    }
                }
                _ => {}
            }
        }
        d.prompts_text.truncate(PROMPTS_CAP);
        d.replies_text.truncate(REPLIES_CAP);
        d.bytes_read = offset - from.unwrap_or(0);
        d.marker = offset.to_string();
        Ok(d)
    }

    fn owns_transcript(&self, path: &Path) -> bool {
        path.starts_with(self.dir.join("logs"))
    }

    fn display_prompt(&self, raw: &str) -> Option<String> {
        let t = raw.trim();
        (!t.is_empty() && !t.starts_with('!')).then(|| t.to_string())
    }
    fn clean_recap(&self, recap: &str) -> String {
        let t = recap.trim_end();
        t.strip_suffix(RECAP_FOOTER.trim_end())
            .unwrap_or(t)
            .trim_end()
            .to_string()
    }

    fn read_conversation(&self, path: &Path) -> Result<Vec<Message>> {
        let mut out = vec![];
        for line in BufReader::new(std::fs::File::open(path)?)
            .lines()
            .map_while(Result::ok)
        {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            if let [kind @ ("u" | "a"), ts, text] = parts.as_slice() {
                let role = if *kind == "u" { "user" } else { "assistant" };
                out.push(Message::new(role, text, ts.parse().ok(), 0));
            }
        }
        Ok(out)
    }

    fn conversation_tail(&self, path: &Path) -> Result<Vec<TailItem>> {
        Ok(self
            .read_conversation(path)?
            .into_iter()
            .map(|m| TailItem::Text {
                role: m.role,
                text: m.text,
            })
            .collect())
    }

    fn executable(&self) -> String {
        "fakebot".into()
    }
    fn resume(&self, session_id: &str) -> Invocation {
        Invocation::new("fakebot", &["continue", session_id])
    }
    fn start(&self, req: &StartRequest) -> Result<StartPlan> {
        // this agent cannot pick the id of a new session
        Ok(StartPlan {
            invocation: Invocation {
                program: "fakebot".into(),
                args: vec!["new".into(), req.title.into()],
            },
            session_id: None,
        })
    }
}
