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

mod preview;
use preview::{preview_lines, talk_lines};

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
        match i.notice_state() {
            Some(n) => format!("remind › {}▏  {n}   {note}", i.text),
            None => format!("remind › {}▏  {note}", i.text),
        }
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
mod tests;
