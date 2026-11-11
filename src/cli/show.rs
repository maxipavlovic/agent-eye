//! Subcommands that look at sessions: scan, find, ls, show, hide, resume.

use super::{column_of, folder, session_json, Global};
use crate::columns;
use crate::model::{Column, Session};
use crate::resume;
use crate::scan;
use crate::search;
use crate::util::{ago, fmt_ts, head_chars, now_ms, printable, short_id, tokens_line};
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub fn scan(g: &Global, full: bool, json: bool) -> Result<()> {
    let mut env = g.open()?;
    let st = scan::scan(&mut env.db, &env.cfg, full)?;
    let kinds = env
        .db
        .count_group("SELECT kind, count(*) FROM session GROUP BY kind ORDER BY 2 DESC")?;
    let outs = env
        .db
        .count_group("SELECT kind, count(*) FROM output GROUP BY kind ORDER BY 2 DESC")?;
    if json {
        let m = |v: &[(String, i64)]| -> Value {
            Value::Object(v.iter().map(|(k, n)| (k.clone(), json!(n))).collect())
        };
        println!(
            "{}",
            json!({"stats": st, "sessions_by_kind": m(&kinds), "outputs_by_kind": m(&outs)})
        );
    } else {
        let fmt = |v: &[(String, i64)]| {
            v.iter()
                .map(|(k, n)| format!("{k}={n}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!(
            "scanned {} transcripts ({} changed, {:.1} MB read), {} history prompts, in {} ms",
            st.files_seen,
            st.files_parsed,
            st.bytes_read as f64 / 1e6,
            st.history_prompts,
            st.elapsed_ms
        );
        println!("sessions: {}", fmt(&kinds));
        println!("outputs:  {}", fmt(&outs));
        if env.agents.is_empty() {
            println!("no agent is enabled (see [agents] in config.toml): nothing to scan");
        }
    }
    Ok(())
}

pub fn find(g: &Global, words: &[String], limit: usize, all: bool, json: bool) -> Result<()> {
    let env = g.open()?;
    let hidden = if all {
        Default::default()
    } else {
        crate::hide::hidden_ids(&env.db, &env.cfg)?
    };
    let hits = search::find_without(&env.db, words, limit, all, &hidden, now_ms())?;
    if json {
        let arr: Vec<Value> = hits
            .iter()
            .map(|h| {
                let mut v = session_json(&h.session, h.column, &env.cfg.user, &env.agents);
                v["snippet"] = json!(h.snippet);
                v["score"] = json!(h.score);
                v
            })
            .collect();
        println!("{}", Value::Array(arr));
    } else if hits.is_empty() {
        println!("no sessions match");
    } else {
        for h in &hits {
            let s = &h.session;
            println!(
                "{}  {}  {:<9}  {}",
                short_id(&s.id),
                s.last_activity_at.map(fmt_ts).unwrap_or_default(),
                h.column.as_str(),
                printable(s.title.as_deref().unwrap_or("(untitled)"))
            );
            println!(
                "          {}{}",
                folder(s),
                s.git_branch
                    .as_deref()
                    .map(|b| format!(" · {}", printable(b)))
                    .unwrap_or_default()
            );
            if !h.snippet.is_empty() {
                println!("          {}", printable(&h.snippet));
            }
        }
    }
    Ok(())
}

pub fn ls(g: &Global, column: Option<&str>, all: bool, limit: usize, json: bool) -> Result<()> {
    let env = g.open()?;
    let now = now_ms();
    let hidden = if all {
        Default::default()
    } else {
        crate::hide::hidden_ids(&env.db, &env.cfg)?
    };
    let want = match column {
        Some(c) => Some(Column::parse(c).with_context(|| format!("unknown column '{c}'"))?),
        None => None,
    };
    let with_outputs = env.db.session_ids_with_outputs()?;
    let mut rows: Vec<(Session, Column)> = env
        .db
        .all_sessions()?
        .into_iter()
        .filter(|s| !hidden.contains(&s.id))
        .map(|s| {
            let c = columns::compute(&columns::Input::from_session(
                &s,
                with_outputs.contains(&s.id),
                now,
            ));
            (s, c)
        })
        .filter(|(_, c)| match want {
            Some(w) => *c == w,
            None => *c != Column::Archive,
        })
        .collect();
    rows.sort_by_key(|(s, _)| std::cmp::Reverse(s.last_activity_at.unwrap_or(0)));
    rows.truncate(limit);
    if json {
        println!(
            "{}",
            Value::Array(
                rows.iter()
                    .map(|(s, c)| session_json(s, *c, &env.cfg.user, &env.agents))
                    .collect()
            )
        );
    } else {
        for (s, c) in &rows {
            println!(
                "{}  {:<9}  {:>8}  {}  [{}]",
                short_id(&s.id),
                c.as_str(),
                s.last_activity_at.map(|t| ago(now, t)).unwrap_or_default(),
                printable(s.title.as_deref().unwrap_or("(untitled)")),
                folder(s)
            );
        }
        if rows.is_empty() {
            println!(
                "no sessions (run `{} scan` first?)",
                crate::pick::bin_name()
            );
        }
    }
    Ok(())
}

pub fn hide(g: &Global, id: &str, hide: bool) -> Result<()> {
    let env = g.open()?;
    let id = env.db.resolve_id(id)?;
    env.db.set_hidden(&id, Some(hide))?;
    let s = env.db.session_by_id(&id)?.context("session vanished")?;
    println!(
        "{} {}  {}",
        if hide { "hidden" } else { "shown" },
        short_id(&id),
        printable(s.title.as_deref().unwrap_or("(untitled)"))
    );
    Ok(())
}

pub fn show(g: &Global, id: &str, json: bool, conversation: bool, n: usize) -> Result<()> {
    let env = g.open()?;
    let id = env.db.resolve_id(id)?;
    let s = env.db.session_by_id(&id)?.context("session vanished")?;
    if conversation {
        let agent = env
            .agents
            .of_session(&s)
            .with_context(|| format!("the agent of this session ({}) is not enabled", s.agent))?;
        let all = crate::conversation::read(&s, &env.paths, agent.as_ref())?;
        let page = crate::conversation::page(&all, n.max(1), None);
        if json {
            println!("{}", page.to_json());
            return Ok(());
        }
        if page.more {
            let n = page.next_before.unwrap_or(0);
            println!("({n} earlier message{})\n", if n == 1 { "" } else { "s" });
        }
        let agent_name = agent.short_name().to_lowercase();
        for m in &page.messages {
            let who = if m.role == "user" { "you" } else { &agent_name };
            let tools = if m.tools > 0 {
                format!(" · {} tool calls", m.tools)
            } else {
                String::new()
            };
            let at =
                m.ts.map(|t| format!(" · {}", fmt_ts(t)))
                    .unwrap_or_default();
            println!("{who}{at}{tools}");
            println!(
                "{}{}\n",
                printable(&m.text),
                if m.truncated { " […]" } else { "" }
            );
        }
        return Ok(());
    }
    let col = column_of(&env.db, &s)?;
    let streams = env.db.streams_for(&id)?;
    let outputs = env.db.outputs_for(&id)?;
    if json {
        let mut v = session_json(&s, col, &env.cfg.user, &env.agents);
        v["streams"] = serde_json::to_value(&streams)?;
        v["outputs"] = serde_json::to_value(&outputs)?;
        v["first_prompt"] = json!(s.first_prompt);
        v["last_user_text"] = json!(s.last_user_text);
        v["last_assistant_text"] = json!(s.last_assistant_text);
        println!("{v}");
        return Ok(());
    }
    println!("{}", printable(s.title.as_deref().unwrap_or("(untitled)")));
    println!("id:       {}", printable(&s.id));
    println!(
        "agent:    {}",
        env.agents
            .get(&s.agent)
            .map_or(s.agent.as_str(), |a| a.name())
    );
    println!(
        "column:   {}   kind: {}   transcript: {}",
        col.as_str(),
        s.kind.as_deref().unwrap_or("?"),
        s.transcript_state.as_deref().unwrap_or("?")
    );
    println!(
        "folder:   {}{}",
        folder(&s),
        s.git_branch
            .as_deref()
            .map(|b| format!(" · {}", printable(b)))
            .unwrap_or_default()
    );
    if let Some(t) = &s.tokens {
        println!("tokens:   {}", tokens_line(t));
    }
    println!(
        "time:     {} → {}   prompts: {}   replies: {}",
        s.started_at.map(fmt_ts).unwrap_or_default(),
        s.last_activity_at.map(fmt_ts).unwrap_or_default(),
        s.prompt_count,
        s.reply_count
    );
    if let Some(p) = &s.parent_session_id {
        println!("parent:   {}", printable(p));
    }
    if let Some(v) = &s.state_line {
        println!("state:    {}", printable(v));
    }
    if let Some(v) = s.next_line() {
        println!("next:     {}", printable(&env.cfg.user.as_you(&v)));
    }
    if let Some(r) = crate::remind::of_session(&s, now_ms()) {
        println!("remind:   {}", printable(&r.summary(now_ms())));
    }
    if let Some(v) = &s.recap {
        println!("recap:    {}", printable(v));
    }
    if let Some(v) = &s.first_prompt {
        println!("first:    {}", printable(&head_chars(v, 300)));
    }
    for st in &streams {
        println!(
            "stream:   [{}] {} — {}{}",
            st.kind,
            printable(st.title.as_deref().unwrap_or(&st.key)),
            printable(st.status.as_deref().unwrap_or("")),
            st.link
                .as_deref()
                .map(|l| format!(" ({})", printable(l)))
                .unwrap_or_default()
        );
    }
    if let Some(line) = env.agents.resume_line(&s) {
        println!("resume:   {}", printable(&line));
    }
    Ok(())
}

pub fn resume(g: &Global, id: &str, dry_run: bool, fork: bool) -> Result<()> {
    let env = g.open()?;
    let id = env.db.resolve_id(id)?;
    let s = env.db.session_by_id(&id)?.context("session vanished")?;
    let (agent, _) = env.agents.runnable(&s).map_err(anyhow::Error::msg)?;
    let line = if fork {
        env.agents
            .fork_line(&s)
            .with_context(|| format!("{} cannot fork a session", agent.name()))?
    } else {
        env.agents
            .resume_line(&s)
            .context("this session has no resume command")?
    };
    if dry_run {
        println!("{line}");
    } else {
        crate::archive::restore_if_missing(&env.db, &env.paths, &env.agents, &s)?;
        let plan = resume::plan_command(line, &resume::target(&env.cfg, &env.paths));
        println!("{}", plan.command);
        println!("{}", resume::execute(&plan)?);
    }
    Ok(())
}
