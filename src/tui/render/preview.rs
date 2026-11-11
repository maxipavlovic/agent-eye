//! The preview pane: a row's details and the transcript's talk, as lines to draw.

use super::*;

pub(super) fn preview_lines<'a>(app: &App, r: &Row, pal: &Pal) -> Vec<Line<'a>> {
    let mut v: Vec<Line> = vec![];
    let label = |t: &str| Span::styled(format!("{t} "), pal.dim());
    v.push(Line::styled(r.title.clone(), pal.bold()));
    let mut where_ = vec![];
    if !r.project.is_empty() {
        where_.push(r.project.clone());
    }
    if let Some(f) = &r.folder {
        where_.push(home_rel(f));
    }
    if let Some(b) = r
        .session
        .as_ref()
        .and_then(|s| s.git_branch.as_deref())
        .filter(|b| !b.is_empty() && *b != "HEAD")
    {
        where_.push(b.to_string());
    }
    if !where_.is_empty() {
        v.push(Line::styled(where_.join(" · "), pal.dim()));
    }
    let who = if app.agents.len() > 1 {
        format!(" · {}", agent_name(app, r))
    } else {
        String::new()
    };
    v.push(Line::from(vec![
        Span::styled(
            format!("{} {}", pick::mark(r.column), pick::column_label(r.column)),
            pal.mark(r.column),
        ),
        Span::styled(
            format!(
                "{who} · {} ago",
                pick::short_age(app.now, r.last).replace("now", "0m")
            ),
            pal.dim(),
        ),
    ]));
    v.push(Line::raw(""));
    if let Some(rem) = &r.remind {
        v.push(Line::styled(rem.summary(app.now), pal.fg(Color::Yellow)));
    }
    match (&r.session, &r.idea) {
        (Some(s), _) => {
            if s.live_status.is_some() {
                v.push(Line::styled(
                    "Open now in another terminal",
                    pal.fg(Color::Green),
                ));
            }
            if let Some(line) = crate::worked::summary::session_line(&app.db.conn, &s.id, app.now) {
                v.push(Line::styled(format!("worked {line}"), pal.dim()));
            }
            for t in activity::tasks(s, app.now) {
                v.push(Line::from(vec![
                    label("Running"),
                    Span::raw(format!("background: {}", activity::task_line(t, app.now))),
                ]));
            }
            for l in activity::loops(s, app.now) {
                v.push(Line::from(vec![
                    label("Running"),
                    Span::raw(activity::loop_line(l, app.now)),
                ]));
            }
            if let Some(url) = activity::remote_url(s) {
                v.push(Line::from(vec![
                    label("Remote control on"),
                    Span::raw(url.to_string()),
                ]));
            }
            let state = s
                .state_line
                .as_deref()
                .filter(|t| !t.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    s.recap
                        .as_deref()
                        .filter(|t| !t.trim().is_empty())
                        .map(|t| crate::util::head_chars(t.trim(), 220))
                });
            if let Some(t) = state {
                v.push(Line::from(vec![label("State"), Span::raw(t)]));
            }
            if let Some(w) = s.waiting_for() {
                v.push(Line::from(vec![
                    label("Next (you)"),
                    Span::raw(w.to_string()),
                ]));
            } else if let Some(ask) = s.live_ask() {
                v.push(Line::from(vec![label("Next (you)"), Span::raw(ask)]));
            } else if let Some(n) = s.next_step.as_deref().filter(|t| !t.trim().is_empty()) {
                let who = match r.column {
                    Column::NeedsYou => "you",
                    Column::Waiting => "someone else",
                    Column::Running => app.agents.short_name(&s.agent),
                    _ => match s.ai_status.as_deref() {
                        Some("needs_you") => "you",
                        Some("waiting_on_others") => "someone else",
                        _ => "you",
                    },
                };
                // The label already says who acts, so the owner prefix of the line is dropped.
                let agent = app.agents.short_name(&s.agent);
                let n = app.user.without_you(n);
                let n = n
                    .strip_prefix(agent)
                    .and_then(|r| r.strip_prefix(':'))
                    .map_or(n, str::trim_start);
                v.push(Line::from(vec![
                    label(&format!("Next ({who})")),
                    Span::raw(n.to_string()),
                ]));
            }
            let topics: Vec<_> = app
                .detail
                .streams
                .iter()
                .filter(|x| x.kind == "topic")
                .collect();
            if !topics.is_empty() {
                v.push(Line::raw(""));
                v.push(Line::styled("Topics", pal.dim()));
                for t in topics.iter().take(6) {
                    v.push(Line::raw(format!(
                        "  {} {} ({})",
                        topic_mark(t.status.as_deref()),
                        t.title.as_deref().unwrap_or(&t.key),
                        t.status.as_deref().unwrap_or("?").replace('_', " ")
                    )));
                }
                if topics.len() > 6 {
                    v.push(Line::styled(
                        format!("  … {} more", topics.len() - 6),
                        pal.dim(),
                    ));
                }
            }
            let outs = &app.detail.outputs;
            if !outs.is_empty() {
                v.push(Line::raw(""));
                let mut counts: Vec<(String, usize)> = vec![];
                for o in outs {
                    match counts.iter_mut().find(|(k, _)| *k == o.kind) {
                        Some(c) => c.1 += 1,
                        None => counts.push((o.kind.clone(), 1)),
                    }
                }
                let c = counts
                    .iter()
                    .map(|(k, n)| format!("{n} {k}"))
                    .collect::<Vec<_>>()
                    .join(" · ");
                v.push(Line::from(vec![label("Outputs"), Span::raw(c)]));
                if let Some(o) = outs.iter().find(|o| o.kind == "pr") {
                    v.push(Line::raw(format!(
                        "  {}",
                        o.url.as_deref().unwrap_or(&o.value)
                    )));
                }
                let jira: Vec<&str> = outs
                    .iter()
                    .filter(|o| o.kind == "jira")
                    .take(4)
                    .map(|o| o.value.as_str())
                    .collect();
                if !jira.is_empty() {
                    v.push(Line::raw(format!("  {}", jira.join(" "))));
                }
            }
            let kids: Vec<_> = app
                .detail
                .streams
                .iter()
                .filter(|x| x.kind == "child")
                .collect();
            if !kids.is_empty() {
                let done = kids
                    .iter()
                    .filter(|c| c.status.as_deref() == Some("done"))
                    .count();
                v.push(Line::from(vec![
                    label("Sub-agents"),
                    Span::raw(format!("{} ({} done)", kids.len(), done)),
                ]));
            }
            if let Some(t) = &s.tokens {
                v.push(Line::from(vec![label("Tokens"), Span::raw(tokens_line(t))]));
            }
            v.push(Line::raw(""));
            match app.agents.resume_line(s) {
                Some(line) => v.push(Line::from(vec![
                    label("Resume"),
                    Span::raw(shown_line(line, std::env::var("HOME").ok().as_deref())),
                ])),
                // a session without a transcript has its own note below
                None if s.kind.as_deref() != Some("history_only") => {
                    if let Err(why) = app.agents.runnable(s) {
                        v.push(Line::styled(why, pal.dim()));
                    }
                }
                None => {}
            }
            if s.kind.as_deref() == Some("history_only") {
                v.push(Line::styled(
                    "no transcript any more: this session cannot be resumed",
                    pal.dim(),
                ));
            } else if s.transcript_state.as_deref() != Some("present") && s.archive_path.is_some() {
                v.push(Line::styled(
                    "transcript gone: restored from eye's archive on resume",
                    pal.dim(),
                ));
            }
        }
        (_, Some(i)) => {
            if let Some(n) = i.notes.as_deref().filter(|t| !t.trim().is_empty()) {
                v.push(Line::from(vec![label("Notes"), Span::raw(n.to_string())]));
            }
            if let Some(p) = i.prompt.as_deref().filter(|t| !t.trim().is_empty()) {
                v.push(Line::from(vec![label("Prompt"), Span::raw(p.to_string())]));
            }
            v.push(Line::raw(""));
            let who = app
                .agents
                .resolve(i.agent.as_deref())
                .map_or("the agent", |a| a.short_name());
            if let Some(note) = r.folder_state.as_ref().and_then(|st| st.note()) {
                v.push(Line::styled(note, pal.dim()));
            }
            v.push(Line::styled(
                if i.prompt.is_some() {
                    format!("Enter starts it here: {who} in its folder with the prompt as the first message")
                } else {
                    format!("Enter starts it here: {who} in its folder, waiting for you (the notes are not sent)")
                },
                pal.dim(),
            ));
        }
        _ => {}
    }
    v
}

/// Break `text` into lines of at most `w` columns, at spaces when it can. Control characters
/// are dropped so nothing in a transcript can drive the terminal.
pub(super) fn wrap(text: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let clean: String = text
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| *c == '\n' || !c.is_control())
        .collect();
    let mut out = vec![];
    for line in clean.lines() {
        let (mut cur, mut cw) = (String::new(), 0usize);
        let mut last_sp: Option<usize> = None;
        for ch in line.chars() {
            let c = ch.width().unwrap_or(0);
            if cw + c > w && ch == ' ' {
                out.push(std::mem::take(&mut cur));
                (cw, last_sp) = (0, None);
                continue;
            }
            if cw + c > w {
                match last_sp.take().filter(|&i| i > 0) {
                    Some(i) => {
                        let rest = cur[i..].trim_start().to_string();
                        cur.truncate(i);
                        out.push(cur.trim_end().to_string());
                        cur = rest;
                        cw = cur.width();
                        last_sp = cur.rfind(' ');
                    }
                    None => {
                        out.push(std::mem::take(&mut cur));
                        cw = 0;
                    }
                }
            }
            if ch == ' ' {
                last_sp = Some(cur.len());
            }
            cur.push(ch);
            cw += c;
        }
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The conversation pane: a title line, then the newest messages at the bottom, scrolled
/// `app.talk_scroll` lines up. Clamps the scroll to what exists.
pub(super) fn talk_lines<'a>(app: &mut App, w: usize, h: usize, pal: &Pal) -> Vec<Line<'a>> {
    let mut out: Vec<Line> = vec![];
    let Some(id) = app.current().map(|r| r.id.clone()) else {
        return out;
    };
    let head = |t: String| Line::styled(clip(&t, w), pal.dim());
    let (total, msgs) = match app.talk_cache.get(&id) {
        Some(Ok((n, m))) => (*n, m),
        Some(Err(e)) => return vec![head(format!("Conversation: {e}"))],
        None => return vec![head("Conversation: loading…".into())],
    };
    let who = app
        .current()
        .and_then(|r| r.session.as_ref())
        .map_or("Agent", |s| app.agents.short_name(&s.agent))
        .to_string();
    let mut body: Vec<Line> = vec![];
    for m in msgs {
        let user = m.role == "user";
        let mut label = if user { "You" } else { who.as_str() }.to_string();
        if let Some(t) = m.ts {
            label.push_str(&format!(" · {} ago", pick::short_age(app.now, t)));
        }
        if m.tools > 0 {
            label.push_str(&format!(" · {} tool calls", m.tools));
        }
        let style = if user {
            pal.fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            pal.bold()
        };
        body.push(Line::styled(clip(&label, w), style));
        if !m.text.is_empty() {
            body.extend(wrap(&m.text, w).into_iter().map(Line::raw));
        }
        if m.truncated {
            body.push(Line::styled("[…]", pal.dim()));
        }
        body.push(Line::raw(""));
    }
    body.pop();
    let shown = msgs.len();
    let title = if shown < total {
        format!("Conversation · last {shown} of {total} · Shift-Up/Down scroll")
    } else {
        format!("Conversation · {total} messages · Shift-Up/Down scroll")
    };
    out.push(head(if total == 0 {
        "Conversation: nothing to show".into()
    } else {
        title
    }));
    let room = h.saturating_sub(1);
    let max_scroll = body.len().saturating_sub(room);
    app.talk_scroll = app.talk_scroll.min(max_scroll);
    let end = body.len() - app.talk_scroll;
    let start = end.saturating_sub(room);
    let pad = room.saturating_sub(end - start);
    out.extend(std::iter::repeat_with(|| Line::raw("")).take(pad));
    out.extend(body.drain(start..end));
    out
}

pub(super) fn topic_mark(status: Option<&str>) -> &'static str {
    match status {
        Some("done") => "✓",
        Some("needs_you") => "●",
        Some("waiting_on_others") => "◷",
        Some("active") => "▶",
        _ => "‖",
    }
}
