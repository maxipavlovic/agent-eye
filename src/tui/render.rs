//! Drawing: the list, the preview and the conversation pane.

use super::state::App;
use crate::activity;
use crate::folder::FolderState;
use crate::model::Column;
use crate::pick::{self, Row};
use crate::util::{home_rel, tokens_line};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

// ---- drawing ----

pub(super) struct Pal {
    color: bool,
}

impl Pal {
    fn fg(&self, c: Color) -> Style {
        if self.color {
            Style::default().fg(c)
        } else {
            Style::default()
        }
    }
    fn dim(&self) -> Style {
        Style::default().add_modifier(Modifier::DIM)
    }
    fn bold(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }
    fn mark(&self, c: Column) -> Style {
        match c {
            Column::NeedsYou => self.fg(Color::Red),
            Column::Running => self.fg(Color::Green),
            Column::Waiting => self.fg(Color::Blue),
            Column::Ideas => self.fg(Color::Yellow),
            Column::Paused | Column::Done | Column::Archive => self.dim(),
        }
    }
}

pub(super) fn clip(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('…');
    out
}

pub(super) fn pad(s: &str, w: usize) -> String {
    let n = s.width();
    format!("{s}{}", " ".repeat(w.saturating_sub(n)))
}

/// The agent of a row by its short name, and the width of the column for it.
type AgentLabel<'a> = Option<(&'a str, usize)>;

/// The short name of the agent a row belongs to (an idea with none is the default agent's).
fn agent_name<'a>(app: &'a App, r: &'a Row) -> &'a str {
    match (&r.session, &r.idea) {
        (Some(s), _) => app.agents.short_name(&s.agent),
        (_, Some(i)) => app
            .agents
            .resolve(i.agent.as_deref())
            .map_or("", |a| a.short_name()),
        _ => "",
    }
}

/// The width of the agent column: the longest short name, when there is more than one agent to tell
/// apart.
fn agent_column(app: &App) -> usize {
    if app.agents.len() < 2 {
        return 0;
    }
    app.agents
        .iter()
        .map(|a| a.short_name().width())
        .max()
        .unwrap_or(0)
        .min(8)
}

pub(super) fn row_line<'a>(
    r: &Row,
    width: usize,
    now: i64,
    pal: &Pal,
    words: &[String],
    agent: AgentLabel,
    new_folder: bool,
) -> Line<'a> {
    let age = pick::short_age(now, r.last);
    let age_w = 4;
    let proj_w = if width >= 60 {
        14
    } else if width >= 40 {
        10
    } else {
        0
    };
    let proj = if proj_w > 0 {
        // a plus marks an idea whose folder does not exist yet
        let name = if new_folder {
            format!("+{}", r.project)
        } else {
            r.project.clone()
        };
        clip(&name, proj_w)
    } else {
        String::new()
    };
    let agent_w = agent.map_or(0, |(_, w)| if width >= 60 { w + 1 } else { 0 });
    // a reminder that is near or due: the chip takes its room from the title
    let chip = if width >= 60 {
        super::remind::chip(r, now).map(|c| format!("{c} "))
    } else {
        None
    };
    let chip_w = chip.as_deref().map_or(0, |c| c.width());
    let fixed = 2 + if proj_w > 0 { proj_w + 1 } else { 0 } + agent_w + age_w + 1 + chip_w;
    let title_w = width.saturating_sub(fixed).max(4);
    // what the session does on its own sits at the end of the title's room
    let marks = r
        .session
        .as_ref()
        .filter(|_| width >= 60)
        .map(|s| activity::marker(s, now))
        .unwrap_or_default();
    let marks_w = if marks.is_empty() {
        0
    } else {
        marks.width() + 1
    };
    let title = clip(&r.title, title_w.saturating_sub(marks_w).max(4));
    let mut spans = vec![Span::styled(
        format!("{} ", pick::mark(r.column)),
        pal.mark(r.column),
    )];
    let base = if matches!(r.column, Column::Done | Column::Archive) {
        pal.dim()
    } else {
        Style::default()
    };
    spans.extend(highlight(&title, words, base, pal.bold()));
    let tw = title.width();
    if marks.is_empty() {
        spans.push(Span::raw(" ".repeat(title_w.saturating_sub(tw) + 1)));
    } else {
        spans.push(Span::raw(
            " ".repeat(title_w.saturating_sub(marks_w + tw) + 1),
        ));
        spans.push(Span::styled(marks, pal.dim()));
        spans.push(Span::raw(" "));
    }
    if let Some(c) = chip {
        spans.push(Span::styled(c, pal.fg(Color::Yellow)));
    }
    if let (Some((name, w)), true) = (agent, agent_w > 0) {
        spans.push(Span::styled(pad(&clip(name, w), agent_w), pal.dim()));
    }
    if proj_w > 0 {
        spans.push(Span::styled(pad(&proj, proj_w + 1), pal.dim()));
    }
    spans.push(Span::styled(format!("{age:>age_w$}"), pal.dim()));
    Line::from(spans)
}

/// Split `text` so that each query word (case-insensitive) is bold.
pub(super) fn highlight<'a>(
    text: &str,
    words: &[String],
    base: Style,
    hit: Style,
) -> Vec<Span<'a>> {
    let lc = text.to_lowercase();
    // Lowercasing can change byte lengths; skip highlighting then.
    if lc.len() != text.len() || words.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let mut marks = vec![false; text.len()];
    for w in words {
        if w.is_empty() {
            continue;
        }
        let mut from = 0;
        while let Some(p) = lc[from..].find(w.as_str()) {
            let a = from + p;
            for m in &mut marks[a..a + w.len()] {
                *m = true;
            }
            from = a + w.len();
        }
    }
    let mut spans = vec![];
    let mut start = 0;
    for (i, _) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if i == text.len() || (i > start && marks[i] != marks[start]) {
            if i > start {
                spans.push(Span::styled(
                    text[start..i].to_string(),
                    if marks[start] { base.patch(hit) } else { base },
                ));
            }
            start = i;
        }
    }
    spans
}

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

/// Draw the whole screen.
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let pal = Pal { color: app.color };
    let [head, body, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    // input line
    let count = format!(
        "{}{}{}",
        app.results.len(),
        if app.all { " all" } else { "" },
        if std::env::var_os("EYE_PICK_TIMING").is_some() {
            format!(" {:.1}ms", app.last_ms)
        } else {
            String::new()
        }
    );
    let prompt = "› ";
    let input_w = (head.width as usize).saturating_sub(count.width() + 1);
    let shown = clip_tail(&app.query, input_w.saturating_sub(prompt.width() + 1));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prompt, pal.fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(shown.clone()),
        ])),
        head,
    );
    f.render_widget(
        Paragraph::new(Line::styled(count, pal.dim())).alignment(ratatui::layout::Alignment::Right),
        head,
    );
    f.set_cursor_position(Position::new(
        head.x + (prompt.width() + shown.width()) as u16,
        head.y,
    ));

    // body
    let preview_on = app.show_preview && body.height >= 8 && body.width >= 40;
    let wide = body.width >= 100;
    let (list_area, prev_area) = if !preview_on {
        (body, None)
    } else if wide {
        let [l, p] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
            .areas(body);
        (l, Some(p))
    } else {
        let ph = (body.height / 2)
            .clamp(5, 12)
            .min(body.height.saturating_sub(4));
        let [l, p] = Layout::vertical([Constraint::Min(3), Constraint::Length(ph)]).areas(body);
        (l, Some(p))
    };

    let words: Vec<String> = app
        .query
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    let lw = list_area.width as usize;
    let label_w = agent_column(app);
    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|&i| {
            let r = &app.idx.rows[i];
            let label = (label_w > 0).then(|| (agent_name(app, r), label_w));
            let new_folder = matches!(&r.folder_state, Some(st) if *st != FolderState::Exists);
            ListItem::new(row_line(r, lw, app.now, &pal, &words, label, new_folder))
        })
        .collect();
    app.page = (list_area.height as usize).saturating_sub(1).max(1);
    let list = List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    if app.results.is_empty() {
        let t = if app.query.trim().is_empty() {
            format!(
                "no sessions yet: start one in {}, then run eye again",
                app.agents.names_or()
            )
        } else {
            "no session matches".into()
        };
        f.render_widget(
            Paragraph::new(Line::styled(format!("  {t}"), pal.dim())).wrap(Wrap { trim: false }),
            list_area,
        );
    } else {
        f.render_stateful_widget(list, list_area, &mut app.list_state);
    }

    if let Some(p) = prev_area {
        let block = if wide {
            Block::default()
                .borders(Borders::LEFT)
                .border_style(pal.dim())
        } else {
            Block::default()
                .borders(Borders::TOP)
                .border_style(pal.dim())
        };
        let inner = block.inner(p);
        f.render_widget(block, p);
        let inner = Rect {
            x: inner.x + if wide { 1 } else { 0 },
            width: inner.width.saturating_sub(if wide { 1 } else { 0 }),
            ..inner
        };
        if app.talk {
            app.ensure_talk();
            let lines = talk_lines(app, inner.width as usize, inner.height as usize, &pal);
            f.render_widget(Paragraph::new(lines), inner);
        } else {
            let lines = match app.current() {
                Some(r) => preview_lines(app, r, &pal),
                None => vec![],
            };
            f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        }
    }

    // hint / message
    let hint_full = "↑↓ move   Enter resume   Ctrl-F fork   Ctrl-T talk   Ctrl-R remind   Ctrl-Y copy   Ctrl-X hide   Tab preview   Ctrl-A hidden+archive   Esc quit";
    let hint_mid =
        "↑↓  Enter resume  ^F fork  ^T talk  ^R remind  ^Y copy  ^X hide  Tab preview  ^A all  Esc quit";
    let hint_short = "Enter resume  ^F fork  ^T talk  ^Y copy  ^X hide  Esc quit";
    let hint_min = "Enter resume  ^F fork  ^T talk  Esc quit";
    let w = foot.width as usize;
    let remind_text = app.remind.as_ref().map(|i| {
        let note = i.error.as_deref().unwrap_or(super::remind::HINT);
        format!("remind › {}▏  {note}", i.text)
    });
    let text = match (&remind_text, &app.msg) {
        (Some(t), _) => t.clone(),
        (None, Some(m)) => m.clone(),
        (None, None) if hint_full.width() <= w => hint_full.into(),
        (None, None) if hint_mid.width() <= w => hint_mid.into(),
        (None, None) if hint_short.width() <= w => hint_short.into(),
        (None, None) => hint_min.into(),
    };
    f.render_widget(
        Paragraph::new(Line::styled(clip(&text, w), pal.dim())),
        foot,
    );
}

pub(super) fn clip_tail(s: &str, w: usize) -> String {
    if s.width() <= w {
        return s.to_string();
    }
    let mut out: Vec<char> = vec![];
    let mut used = 1;
    for ch in s.chars().rev() {
        let cw = ch.width().unwrap_or(0);
        if used + cw > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
}

/// A resume line as the preview shows it: the home folder as `~`, still a valid shell line. What
/// runs (Enter, Ctrl-Y, Ctrl-G) keeps the full path.
fn shown_line(line: String, home: Option<&str>) -> String {
    let prefix = match home {
        Some(h) if !h.is_empty() => format!("cd '{h}/"),
        _ => return line,
    };
    match line.strip_prefix(prefix.as_str()) {
        Some(rest) => format!("cd ~/'{rest}"),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use crate::agents::codex::Codex;
    use crate::agents::{AgentRef, Agents};
    use crate::tui::render_text;
    use crate::tui::test_support::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn the_preview_shortens_the_home_folder() {
        let line = "cd '/Users/you/code/a b' && claude --resume abcd-1".to_string();
        assert_eq!(
            shown_line(line.clone(), Some("/Users/you")),
            "cd ~/'code/a b' && claude --resume abcd-1"
        );
        assert_eq!(shown_line(line.clone(), Some("/Users/other")), line);
        assert_eq!(shown_line(line.clone(), None), line);
    }

    #[test]
    fn frames_at_three_sizes() {
        for (w, h) in [(80, 24), (120, 40), (200, 50)] {
            let mut a = app("");
            let t = render_text(&mut a, w, h).unwrap();
            println!("---- {w}x{h}, empty query ----\n{t}");
            for l in t.lines() {
                assert!(l.chars().count() <= w as usize);
            }
            assert!(t.contains("Esc"));
            let mut a = app("catalog");
            let t = render_text(&mut a, w, h).unwrap();
            println!("---- {w}x{h}, query 'catalog' ----\n{t}");
            assert!(t.lines().nth(1).unwrap().contains("Catalog migration"));
        }
    }

    /// The picker over Claude Code and Codex CLI, its second row a Codex session.
    fn mixed_app() -> App {
        let mut a = app("");
        let claude: AgentRef = std::sync::Arc::new(Claude::new("/nonexistent/c".into()));
        let codex: AgentRef = std::sync::Arc::new(Codex::new("/nonexistent/x".into()));
        a.agents = Agents::from_vec(vec![claude, codex]);
        a.idx.rows[1].session.as_mut().unwrap().agent = "codex".into();
        a
    }

    #[test]
    fn with_several_agents_the_rows_and_the_preview_name_the_agent() {
        let mut a = mixed_app();
        let t = render_text(&mut a, 80, 30).unwrap();
        let row = |t: &str, title: &str| t.lines().find(|l| l.contains(title)).unwrap().to_string();
        let codex = row(&t, "Weekly report skill");
        let claude = row(&t, "Catalog migration");
        assert!(
            codex.contains("Codex") && !codex.contains("Claude"),
            "{codex}"
        );
        assert!(
            claude.contains("Claude") && !claude.contains("Codex"),
            "{claude}"
        );
        assert!(
            row(&t, "Try the Zephyr").contains("Claude"),
            "an idea belongs to the default agent"
        );
        for l in t.lines() {
            assert!(l.chars().count() <= 80, "{l}");
        }
        // the preview of the selected row says it too
        a.key(key(KeyCode::Down));
        let t = render_text(&mut a, 80, 30).unwrap();
        assert!(t.contains("Running · Codex · "), "{t}");
        // a narrow list has no room for the column and stays readable
        let t = render_text(&mut a, 50, 20).unwrap();
        assert!(!row(&t, "Weekly report").contains("Codex"), "{t}");
    }

    #[test]
    fn the_preview_says_the_days_worked_once_they_are_counted() {
        use crate::worked::{store, WorkEvent};
        let mut a = app("");
        let t = render_text(&mut a, 100, 30).unwrap();
        assert!(!t.contains("worked "), "{t}");
        let at = a.now - 3_600_000;
        let events = [
            WorkEvent {
                ts: at,
                prompt: true,
            },
            WorkEvent {
                ts: at + 600_000,
                prompt: false,
            },
        ];
        let id = a.current().unwrap().id.clone();
        store::record(&a.db.conn, &id, &events, true).unwrap();
        let t = render_text(&mut a, 100, 30).unwrap();
        assert!(t.contains("worked 1 day · 10 m · last "), "{t}");
    }

    #[test]
    fn with_one_agent_nothing_is_added_to_the_rows_or_the_preview() {
        let mut a = app("");
        let t = render_text(&mut a, 80, 30).unwrap();
        assert!(!t.contains("Codex"));
        let list: String = t
            .lines()
            .filter(|l| l.contains("Weekly report") || l.contains("Catalog migration"))
            .collect();
        assert!(!list.contains("Claude"), "{list}");
        assert!(
            !t.contains("Needs you · Claude") && !t.contains("· Claude ·"),
            "{t}"
        );
    }

    #[test]
    fn an_empty_picker_says_where_sessions_come_from() {
        let mut a = app("");
        a.idx.rows.clear();
        a.research();
        let t = render_text(&mut a, 80, 24).unwrap();
        assert!(
            t.contains("no sessions yet: start one in Claude Code, then run eye again"),
            "{t}"
        );
        // a search with no hit is another message, and a narrow terminal wraps instead of cutting
        let mut a = app("zzzz");
        let t = render_text(&mut a, 80, 24).unwrap();
        assert!(t.contains("no session matches") && !t.contains("no sessions yet"));
        let mut a = app("");
        a.idx.rows.clear();
        a.research();
        let t = render_text(&mut a, 40, 20).unwrap();
        assert!(t.contains("then run eye again"), "{t}");
    }

    #[test]
    fn the_preview_says_why_a_session_cannot_be_resumed() {
        let mut a = app("");
        a.idx.rows[0].session.as_mut().unwrap().cwd = Some("/tmp/a\nb".into());
        let t = render_text(&mut a, 200, 40).unwrap();
        assert!(t.contains("control"), "{t}");
        assert!(!t.contains("Resume "), "{t}");
    }

    #[test]
    fn live_session_preview_says_so() {
        let mut a = app("");
        a.idx.rows[0].session.as_mut().unwrap().live_status = Some("busy".into());
        let t = render_text(&mut a, 120, 40).unwrap();
        assert!(t.contains("Open now in another terminal"));
    }

    #[test]
    fn the_next_line_does_not_repeat_its_owner() {
        for (stored, shown) in [
            ("You: review the PR", "review the PR"),
            ("Sam: review the PR", "review the PR"),
            ("Claude: fix the test", "fix the test"),
            ("Dana: approve the budget", "Dana: approve the budget"),
        ] {
            let mut a = app("");
            a.user = crate::config::UserCfg { name: "Sam".into() };
            a.idx.rows[0].session.as_mut().unwrap().next_step = Some(stored.into());
            let t = render_text(&mut a, 140, 40).unwrap();
            let line = t.lines().find(|l| l.contains("Next (")).unwrap_or("");
            assert!(line.contains(shown), "{stored}: {line}");
            assert!(
                !line.contains("You:") && !line.contains("Sam:") && !line.contains("Claude:"),
                "{stored}: {line}"
            );
        }
    }

    #[test]
    fn conversation_view_reports_missing_transcripts_and_ideas() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let (_tmp, mut a) = talk_app(1);
        a.key(ctrl('t'));
        a.key(key(KeyCode::Down));
        let t = render_text(&mut a, 120, 20).unwrap();
        assert!(
            t.contains("Conversation: no transcript or archived copy"),
            "{t}"
        );
        for c in "hyd".chars() {
            a.key(key(KeyCode::Char(c)));
        }
        let t = render_text(&mut a, 120, 20).unwrap();
        assert!(t.contains("an idea has no conversation"), "{t}");
    }

    #[test]
    fn preview_shows_the_token_line() {
        let mut a = app("");
        a.idx.rows[0].session.as_mut().unwrap().tokens = Some(crate::model::Tokens {
            input: 1_000_000,
            output: 500_000,
            cache_write: 300_000,
            cache_read: 100_000,
            subagents_total: 410_000,
        });
        let t = render_text(&mut a, 120, 40).unwrap();
        assert!(t.contains("Tokens 2.3M (sub-agents 410k)"), "{t}");
    }

    #[test]
    fn wrap_breaks_at_spaces_and_drops_control_characters() {
        assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(wrap("x\u{1b}[31mred", 20), ["x[31mred"]);
        assert_eq!(wrap("", 5), [""]);
        for l in wrap("日本語のテキストを折り返す テスト", 6) {
            assert!(l.width() <= 6, "{l}");
        }
    }

    #[test]
    fn narrow_and_tiny_terminals_do_not_panic() {
        for (w, h) in [(20, 5), (40, 8), (60, 12), (1, 1), (100, 3)] {
            let mut a = app("cat");
            let t = render_text(&mut a, w, h).unwrap();
            for l in t.lines() {
                assert!(l.chars().count() <= w as usize);
            }
        }
    }
}
