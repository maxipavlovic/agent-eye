//! One session as a file people can read: a self-contained HTML page (no network, its own CSS in
//! the calm ink style of the board) or Markdown. Secrets are masked the way the summaries mask
//! them; tool calls are folded to one line per turn.

use crate::agents::Agents;
use crate::conversation::Message;
use crate::db::Db;
use crate::model::{Output, Session};
use crate::paths::Paths;
use crate::summarize::mask_secrets;
use crate::util::{fmt_ts, now_ms, short_id};
use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Html,
    Markdown,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Html => "html",
            Format::Markdown => "md",
        }
    }
}

/// One session of the database as a page: its file name and its text. The conversation is read
/// from the transcript, or from eye's archive copy when the agent deleted it; a session with
/// neither gets a page that says so.
pub fn export_session(
    db: &Db,
    paths: &Paths,
    agents: &Agents,
    id: &str,
    format: Format,
) -> Result<(String, String)> {
    let id = db.resolve_id(id)?;
    let s = db.session_by_id(&id)?.context("no such session")?;
    let column = crate::columns::compute(&crate::columns::Input::from_session(
        &s,
        !db.outputs_for(&id)?.is_empty(),
        now_ms(),
    ));
    let agent = agents.of_session(&s);
    let conv = match agent {
        Some(a) => crate::conversation::read(&s, paths, a.as_ref()).map_err(|e| format!("{e:#}")),
        None => Err(format!(
            "the agent of this session ({}) is not enabled",
            s.agent
        )),
    };
    let text = render(
        &s,
        column.as_str(),
        conv,
        &db.outputs_for(&id)?,
        agent.map_or(s.agent.as_str(), |a| a.name()),
        format,
    );
    let slug: String = s
        .title
        .as_deref()
        .unwrap_or("session")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|w| !w.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    Ok((
        format!("eye-{}-{slug}.{}", short_id(&id), format.ext()),
        text,
    ))
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// The parts of a session's export, with every text masked.
struct Parts {
    title: String,
    folder: String,
    started: String,
    last: String,
    state: String,
    agent: String,
    id: String,
    messages: Vec<(bool, String, String, u32, bool)>,
    note: Option<String>,
    outputs: Vec<(String, String, Option<String>)>,
}

fn parts(
    s: &Session,
    column: &str,
    conv: Result<Vec<Message>, String>,
    outputs: &[Output],
    agent_name: &str,
) -> Parts {
    let m = |t: &str| mask_secrets(t);
    let mut state = column.to_string();
    if let Some(t) = s.transcript_state.as_deref() {
        state.push_str(&format!(" · transcript {t}"));
    }
    let (messages, note) = match conv {
        Ok(v) => (
            v.into_iter()
                .map(|x| {
                    (
                        x.role == "user",
                        x.text,
                        x.ts.map(fmt_ts).unwrap_or_default(),
                        x.tools,
                        x.truncated,
                    )
                })
                .collect(),
            None,
        ),
        Err(e) => (vec![], Some(m(&e))),
    };
    Parts {
        title: m(s.title.as_deref().unwrap_or("(untitled)")),
        folder: m(s.cwd.as_deref().unwrap_or("")),
        started: s.started_at.map(fmt_ts).unwrap_or_default(),
        last: s.last_activity_at.map(fmt_ts).unwrap_or_default(),
        state,
        agent: agent_name.to_string(),
        id: s.id.clone(),
        messages,
        note,
        outputs: outputs
            .iter()
            .map(|o| (o.kind.clone(), m(&o.value), o.url.as_deref().map(m)))
            .collect(),
    }
}

/// The export of one session. `conv` is the conversation (already masked, as the reader makes it)
/// or why there is none.
pub fn render(
    s: &Session,
    column: &str,
    conv: Result<Vec<Message>, String>,
    outputs: &[Output],
    agent_name: &str,
    format: Format,
) -> String {
    let p = parts(s, column, conv, outputs, agent_name);
    match format {
        Format::Html => html(&p),
        Format::Markdown => markdown(&p),
    }
}

fn who(p: &Parts, user: bool) -> String {
    if user {
        "You".into()
    } else {
        p.agent.clone()
    }
}

fn tools_line(n: u32) -> String {
    format!("{n} tool call{}", if n == 1 { "" } else { "s" })
}

fn markdown(p: &Parts) -> String {
    let mut o = format!("# {}\n\n", p.title.replace('\n', " "));
    o.push_str(&format!(
        "- Folder: `{}`\n- Agent: {}\n- Started: {}\n- Last activity: {}\n- State: {}\n- Session: `{}`\n\n",
        p.folder, p.agent, p.started, p.last, p.state, p.id
    ));
    o.push_str("## Conversation\n\n");
    if let Some(n) = &p.note {
        o.push_str(&format!("_No conversation: {n}_\n\n"));
    }
    for (user, text, ts, tools, cut) in &p.messages {
        o.push_str(&format!("### {}{}\n\n", who(p, *user), at(ts)));
        if !text.is_empty() {
            o.push_str(text);
            o.push_str(if *cut { " […]\n\n" } else { "\n\n" });
        }
        if *tools > 0 {
            o.push_str(&format!("_{}_\n\n", tools_line(*tools)));
        }
    }
    if !p.outputs.is_empty() {
        o.push_str("## Outputs\n\n");
        for (kind, value, url) in &p.outputs {
            match url {
                Some(u) if u != value => o.push_str(&format!("- {kind}: {value} ({u})\n")),
                _ => o.push_str(&format!("- {kind}: {value}\n")),
            }
        }
    }
    o
}

fn at(ts: &str) -> String {
    if ts.is_empty() {
        String::new()
    } else {
        format!(" · {ts}")
    }
}

const CSS: &str = "\
:root{--paper:#f2efe6;--ink:#161616;--mute:#4a473f;--accent:#ff4a1c;--rule:#c9c4b4}\
@media (prefers-color-scheme:dark){:root{--paper:#1e1d1a;--ink:#d6d1c4;--mute:#b0ab9d;--accent:#e8683f;--rule:#46433b}}\
*{box-sizing:border-box}\
body{margin:0;background:var(--paper);color:var(--ink);font:17px/1.55 Inter,system-ui,-apple-system,'Segoe UI',sans-serif}\
main{max-width:46rem;margin:0 auto;padding:2.5rem 1rem 4rem}\
h1{font-size:1.7rem;line-height:1.25;margin:0 0 1rem}\
h2{font-size:.85rem;letter-spacing:.12em;text-transform:uppercase;margin:2.5rem 0 .5rem;border-bottom:2px solid var(--ink);padding-bottom:.25rem}\
dl{display:grid;grid-template-columns:max-content 1fr;gap:.2rem 1rem;margin:0}\
dt{color:var(--mute)}dd{margin:0;overflow-wrap:anywhere}\
.turn{margin:1.25rem 0;padding-left:.9rem;border-left:3px solid var(--rule)}\
.turn.you{border-left-color:var(--accent)}\
.who{font-weight:600}.when{color:var(--mute);font-size:.9rem;margin-left:.5rem}\
.turn pre{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit;margin:.25rem 0}\
.tools{color:var(--mute);font-size:.95rem}\
ul{padding-left:1.2rem}li{overflow-wrap:anywhere}a{color:inherit}\
code{font-family:ui-monospace,Menlo,Consolas,monospace;font-size:.92em}\
.note{color:var(--mute);font-style:italic}";

fn html(p: &Parts) -> String {
    let mut o = String::from("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">");
    o.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    o.push_str(&format!("<title>{}</title>", esc(&p.title)));
    o.push_str(&format!("<style>{CSS}</style></head><body><main>"));
    o.push_str(&format!("<h1>{}</h1><dl>", esc(&p.title)));
    for (k, v) in [
        ("Folder", p.folder.as_str()),
        ("Agent", p.agent.as_str()),
        ("Started", p.started.as_str()),
        ("Last activity", p.last.as_str()),
        ("State", p.state.as_str()),
        ("Session", p.id.as_str()),
    ] {
        if !v.is_empty() {
            o.push_str(&format!("<dt>{k}</dt><dd>{}</dd>", esc(v)));
        }
    }
    o.push_str("</dl><h2>Conversation</h2>");
    if let Some(n) = &p.note {
        o.push_str(&format!(
            "<p class=\"note\">No conversation: {}</p>",
            esc(n)
        ));
    }
    for (user, text, ts, tools, cut) in &p.messages {
        o.push_str(&format!(
            "<section class=\"turn {}\"><span class=\"who\">{}</span>",
            if *user { "you" } else { "agent" },
            esc(&who(p, *user))
        ));
        if !ts.is_empty() {
            o.push_str(&format!("<span class=\"when\">{}</span>", esc(ts)));
        }
        if !text.is_empty() {
            o.push_str(&format!(
                "<pre>{}{}</pre>",
                esc(text),
                if *cut { " […]" } else { "" }
            ));
        }
        if *tools > 0 {
            o.push_str(&format!(
                "<div class=\"tools\">{}</div>",
                tools_line(*tools)
            ));
        }
        o.push_str("</section>");
    }
    if !p.outputs.is_empty() {
        o.push_str("<h2>Outputs</h2><ul>");
        for (kind, value, url) in &p.outputs {
            o.push_str(&format!("<li>{}: ", esc(kind)));
            match url.as_deref().filter(|u| u.starts_with("https://")) {
                Some(u) => o.push_str(&format!(
                    "<a href=\"{}\" rel=\"noreferrer\">{}</a>",
                    esc(u),
                    esc(value)
                )),
                None => o.push_str(&format!("<code>{}</code>", esc(value))),
            }
            o.push_str("</li>");
        }
        o.push_str("</ul>");
    }
    o.push_str("</main></body></html>\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: "abc".into(),
            title: Some("Fix <the> thing".into()),
            cwd: Some("/tmp/demo".into()),
            ..Default::default()
        }
    }

    #[test]
    fn html_is_escaped_masked_and_has_no_external_loads() {
        let secret = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let conv = vec![
            Message::new(
                "user",
                &format!("use {secret} <b>now</b>"),
                Some(1_700_000_000_000),
                0,
            ),
            Message::new("assistant", "done", None, 3),
        ];
        let out = vec![Output {
            id: 1,
            session_id: "abc".into(),
            kind: "pr".into(),
            value: "https://github.com/x/y/pull/1".into(),
            url: Some("https://github.com/x/y/pull/1".into()),
            first_seen_at: None,
            title: None,
        }];
        let h = render(
            &session(),
            "working",
            Ok(conv),
            &out,
            "Claude Code",
            Format::Html,
        );
        assert!(!h.contains(secret), "{h}");
        assert!(h.contains("&lt;b&gt;now&lt;/b&gt;") && h.contains("Fix &lt;the&gt; thing"));
        assert!(h.contains("3 tool calls"));
        for bad in [
            "<link",
            "<script",
            "<img",
            "src=\"http",
            "url(http",
            "@import",
        ] {
            assert!(!h.contains(bad), "{bad}");
        }
        let md = render(
            &session(),
            "working",
            Err("gone".into()),
            &[],
            "Claude Code",
            Format::Markdown,
        );
        assert!(md.starts_with("# Fix <the> thing") && md.contains("No conversation: gone"));
    }
}
