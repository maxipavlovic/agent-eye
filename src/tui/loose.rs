//! What the selected session left behind: the small list (Ctrl-L) with its actions. Every action
//! that changes something asks y/N in the footer first; copying a command does not.

use super::state::App;
use super::Outcome;
use crate::loose::{self, LooseEnd, System};
use crate::platform::clipboard;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the footer says while the list is open.
pub(super) const HINT: &str =
    "↑↓ move   s stop   r remove   c copy command   i ignore   a send back to the agent   Esc close";

/// The list and what waits for a y.
#[derive(Debug)]
pub(super) struct List {
    pub session: String,
    pub items: Vec<LooseEnd>,
    pub sel: usize,
    pub ask: Option<Ask>,
}

/// An action waiting for y/N, on the item at `idx` (or the whole session).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ask {
    Stop(usize),
    Remove(usize),
    Ignore(usize),
    Send,
}

impl App {
    /// Open the list for the selected session.
    pub(super) fn start_loose(&mut self) {
        let Some(r) = self.current() else {
            self.msg = Some("nothing selected".into());
            return;
        };
        if r.session.is_none() {
            self.msg = Some("only a session leaves things behind".into());
            return;
        }
        let id = r.id.clone();
        let items = loose::list(&self.db, Some(&id), false).unwrap_or_default();
        if items.is_empty() {
            self.msg = Some("no leftovers".into());
            return;
        }
        self.msg = None;
        self.show_preview = true;
        self.talk = false;
        self.loose = Some(List {
            session: id,
            items,
            sel: 0,
            ask: None,
        });
    }

    /// Read the list and the preview's line again after something changed.
    fn reload_loose(&mut self) {
        let Some(l) = self.loose.as_mut() else { return };
        l.items = loose::list(&self.db, Some(&l.session), false).unwrap_or_default();
        l.sel = l.sel.min(l.items.len().saturating_sub(1));
        let items = l.items.clone();
        if items.is_empty() {
            self.loose = None;
        }
        self.detail.loose = items;
    }

    pub(super) fn loose_key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let l = self.loose.as_mut()?;
        if let Some(ask) = l.ask.take() {
            // anything but y is a no
            self.msg = None;
            return match k.code {
                KeyCode::Char('c') if ctrl => Some(Outcome::Quit),
                KeyCode::Char('y' | 'Y') => self.loose_do(ask),
                _ => {
                    self.msg = Some("cancelled".into());
                    None
                }
            };
        }
        self.msg = None;
        let item = l.items.get(l.sel).cloned();
        match k.code {
            KeyCode::Char('c') if ctrl => return Some(Outcome::Quit),
            KeyCode::Esc | KeyCode::Char('q') => self.loose = None,
            KeyCode::Up | KeyCode::Char('k') => l.sel = l.sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                l.sel = (l.sel + 1).min(l.items.len().saturating_sub(1))
            }
            KeyCode::Char('s') | KeyCode::Char('r') | KeyCode::Char('i') => {
                let it = item?;
                let sel = l.sel;
                let (ask, text) = match k.code {
                    KeyCode::Char('s') => (
                        Ask::Stop(sel),
                        it.confirm().filter(|_| it.action() == "stop"),
                    ),
                    KeyCode::Char('r') => (
                        Ask::Remove(sel),
                        it.confirm().filter(|_| it.action() == "remove"),
                    ),
                    _ => (
                        Ask::Ignore(sel),
                        Some(format!("Stop listing {}?", it.what())),
                    ),
                };
                match text {
                    Some(t) => {
                        l.ask = Some(ask);
                        self.msg = Some(format!("{t} y/N"));
                    }
                    None => {
                        self.msg = Some(format!(
                            "that does not apply to {} (see the keys below)",
                            it.kind.as_str()
                        ))
                    }
                }
            }
            KeyCode::Char('c') => {
                if let Some(cmd) = item.as_ref().and_then(loose::copy_command) {
                    self.msg = Some(if clipboard::copy(&cmd) {
                        "command copied to the clipboard; eye never runs it for you".into()
                    } else {
                        format!("no clipboard tool; here is the command: {cmd}")
                    });
                } else {
                    self.msg = Some("there is no command to copy for that one".into());
                }
            }
            KeyCode::Char('a') => {
                l.ask = Some(Ask::Send);
                self.msg = Some(
                    "Open this session with a message that lists what it left behind? y/N".into(),
                );
            }
            _ => {}
        }
        None
    }

    fn loose_do(&mut self, ask: Ask) -> Option<Outcome> {
        let l = self.loose.as_ref()?;
        let (session, items) = (l.session.clone(), l.items.clone());
        let key = |i: usize| items.get(i).map(|it| it.key.clone());
        let probe = System::with_roots(self.agents.scratch_roots());
        let said = match ask {
            Ask::Send => return Some(Outcome::SendBack(session)),
            Ask::Stop(i) => key(i).map(|k| loose::stop(&self.db, &probe, &k)),
            Ask::Remove(i) => key(i).map(|k| loose::remove(&self.db, &probe, &k)),
            Ask::Ignore(i) => key(i).map(|k| {
                loose::ignore(&self.db, &k, crate::util::now_ms()).map(|()| "ignored".to_string())
            }),
        };
        self.msg = said.map(|r| match r {
            Ok(s) => s,
            Err(e) => format!("{e:#}"),
        });
        self.reload_loose();
        None
    }
}

/// The list as lines of the preview pane.
pub(super) fn lines(l: &List) -> Vec<String> {
    let mut v = vec!["Leftovers".to_string(), String::new()];
    for (i, it) in l.items.iter().enumerate() {
        let mark = if i == l.sel { "›" } else { " " };
        v.push(format!("{mark} {}", it.line));
        if i == l.sel {
            for extra in it.detail.lines().skip(1) {
                v.push(format!("    {extra}"));
            }
        }
    }
    v
}
