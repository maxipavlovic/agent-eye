//! The terminal: raw mode on /dev/tty, the event loop, and a text frame for tests.

use super::render::draw;
use super::state::App;
use super::Outcome;
use anyhow::{Context, Result};
use crossterm::event::{self, Event};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::Terminal;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

// ---- terminal ----

pub(super) struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        restore_terminal();
    }
}

pub(super) fn restore_terminal() {
    let _ = disable_raw_mode();
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = crossterm::execute!(f, LeaveAlternateScreen, crossterm::cursor::Show);
    }
}

/// Is there a terminal to draw on?
pub fn have_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_ok()
}

/// Run the picker until the user picks or quits. The terminal is restored before this returns,
/// also on panic.
pub fn run(app: &mut App) -> Result<Outcome> {
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("no terminal")?;
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |i| {
        restore_terminal();
        prev(i);
    }));
    enable_raw_mode().context("raw mode")?;
    let _guard = Restore;
    let mut tty = tty;
    crossterm::execute!(tty, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(tty))?;
    let res = event_loop(&mut term, app);
    drop(term);
    res
}

pub(super) fn event_loop<B: Backend>(term: &mut Terminal<B>, app: &mut App) -> Result<Outcome>
where
    B::Error: Send + Sync + 'static,
{
    loop {
        term.draw(|f| draw(f, app))?;
        // Wait for one event, then take everything already queued before redrawing, so fast
        // typing costs one search per frame.
        if !event::poll(Duration::from_secs(3600))? {
            continue;
        }
        loop {
            match event::read()? {
                Event::Key(k) => {
                    if let Some(o) = app.key(k) {
                        return Ok(o);
                    }
                }
                Event::Resize(..) => {
                    term.clear()?;
                }
                _ => {}
            }
            if !event::poll(Duration::from_millis(0))? {
                break;
            }
        }
    }
}

/// Render one frame as text, for tests.
pub fn render_text(app: &mut App, w: u16, h: u16) -> Result<String> {
    let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h))?;
    term.draw(|f| draw(f, app))?;
    let buf = term.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        let mut line = String::new();
        let mut x = 0;
        while x < w {
            let c = &buf[(x, y)];
            line.push_str(c.symbol());
            x += c.symbol().width().max(1) as u16;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    Ok(out)
}
