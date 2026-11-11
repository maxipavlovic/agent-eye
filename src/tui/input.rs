//! Keys: what each key does to the picker.

use super::state::App;
use super::Outcome;
use crate::folder::FolderState;
use crate::pick::{self, Kind};
use crate::platform::clipboard;
use crate::resume;
use crate::util::home_rel;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Lines one Shift-Up / Shift-Down scrolls.
pub(super) const TALK_STEP: usize = 3;

impl App {
    /// Apply one key. `Some` ends the picker.
    pub fn key(&mut self, k: KeyEvent) -> Option<Outcome> {
        if k.kind == KeyEventKind::Release {
            return None;
        }
        if self.remind.is_some() {
            return self.remind_key(k);
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if !matches!(k.code, KeyCode::Enter | KeyCode::Esc) && self.confirm.take().is_some() {
            self.msg = None;
        }
        match k.code {
            KeyCode::Esc if self.confirm.take().is_some() => self.msg = None,
            KeyCode::Esc => return Some(Outcome::Quit),
            KeyCode::Char('c') if ctrl => return Some(Outcome::Quit),
            KeyCode::Enter => {
                if let Some(r) = self.current() {
                    if pick::actionable(r) {
                        match r.folder_state.clone() {
                            Some(FolderState::Missing { creatable: true }) => {
                                if self.confirm.as_deref() != Some(r.id.as_str()) {
                                    let msg = format!(
                                        "Folder {} does not exist — Enter again to create it and start, Esc to cancel",
                                        r.folder.as_deref().map(home_rel).unwrap_or_default()
                                    );
                                    self.confirm = Some(r.id.clone());
                                    self.msg = Some(msg);
                                    return None;
                                }
                            }
                            Some(FolderState::Exists) | None => {}
                            Some(other) => {
                                self.msg = other.note();
                                return None;
                            }
                        }
                        let live = r.session.as_ref().is_some_and(|s| s.live_status.is_some());
                        if live && self.confirm.as_deref() != Some(r.id.as_str()) {
                            self.confirm = Some(r.id.clone());
                            self.msg = Some(
                                "Open in another terminal — Enter again to resume anyway, Esc to cancel"
                                    .into(),
                            );
                            return None;
                        }
                        return Some(Outcome::Pick(r.id.clone()));
                    }
                    self.msg = Some(match r.kind {
                        Kind::Idea => "this idea was already started".into(),
                        Kind::Session => {
                            "no transcript any more; this session cannot be resumed".into()
                        }
                    });
                }
            }
            KeyCode::Up if shift && self.talk => self.talk_scroll += TALK_STEP,
            KeyCode::Down if shift && self.talk => {
                self.talk_scroll = self.talk_scroll.saturating_sub(TALK_STEP)
            }
            KeyCode::Char('t') if ctrl => {
                self.talk = !self.talk;
                self.talk_scroll = 0;
                if self.talk {
                    self.show_preview = true;
                }
            }
            KeyCode::Char('f') if ctrl => {
                if let Some(r) = self.current() {
                    if r.kind == Kind::Session && pick::actionable(r) {
                        return Some(Outcome::Fork(r.id.clone()));
                    }
                    self.msg = Some(match r.kind {
                        Kind::Idea => "an idea cannot be forked; Enter starts it".into(),
                        Kind::Session => {
                            "no transcript any more; this session cannot be forked".into()
                        }
                    });
                }
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::Char('p') if ctrl => self.move_by(-1),
            KeyCode::Char('n') if ctrl => self.move_by(1),
            KeyCode::PageUp => self.move_by(-(self.page as isize)),
            KeyCode::PageDown => self.move_by(self.page as isize),
            KeyCode::Home => self.move_by(isize::MIN / 2),
            KeyCode::End => self.move_by(isize::MAX / 2),
            KeyCode::Tab => self.show_preview = !self.show_preview,
            KeyCode::Char('a') if ctrl => {
                self.all = !self.all;
                self.research();
                self.msg = Some(if self.all {
                    "showing hidden sessions and the archive".into()
                } else {
                    "hidden sessions and the archive are left out".into()
                });
            }
            KeyCode::Char('x') if ctrl => self.toggle_hidden(),
            KeyCode::Char('r') if ctrl => self.start_remind(),
            KeyCode::Char('y') if ctrl => {
                self.msg = Some(match self.current() {
                    Some(r) if r.kind == Kind::Session && pick::actionable(r) => {
                        let cmd = r.session.as_ref().and_then(|s| self.agents.resume_line(s));
                        match cmd {
                            Some(c) if clipboard::copy(&c) => "resume command copied".into(),
                            Some(c) => format!("{}: {c}", resume::NO_CLIPBOARD),
                            None => r
                                .session
                                .as_ref()
                                .and_then(|s| self.agents.runnable(s).err())
                                .unwrap_or_else(|| "nothing to copy".into()),
                        }
                    }
                    Some(_) => "only sessions have a resume command".into(),
                    None => "nothing to copy".into(),
                });
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.research();
            }
            KeyCode::Char('w') if ctrl => {
                let t = self.query.trim_end().len();
                let cut = self.query[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
                self.query.truncate(cut);
                self.research();
            }
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.research();
                }
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.query.push(c);
                self.research();
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Column;
    use crate::tui::render_text;
    use crate::tui::test_support::*;

    #[test]
    fn keys_move_toggle_and_pick() {
        let mut a = app("");
        assert_eq!(a.results.len(), 6, "done is last, no archive");
        assert_eq!(a.current().unwrap().column, Column::NeedsYou);
        a.key(key(KeyCode::Down));
        assert_eq!(a.current().unwrap().column, Column::Running);
        a.key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(a.sel, 0);
        a.key(key(KeyCode::Up));
        assert_eq!(a.sel, 0);
        a.key(key(KeyCode::PageDown));
        assert_eq!(a.sel, 5);
        a.key(key(KeyCode::Tab));
        assert!(!a.show_preview);
        assert_eq!(
            a.key(key(KeyCode::Enter)),
            Some(Outcome::Pick("eeeeeeee-0000-4000-8000-000000000005".into()))
        );
        for c in "hyd".chars() {
            a.key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.current().unwrap().id, "idea:1");
        a.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(a.query, "");
        assert_eq!(a.key(key(KeyCode::Esc)), Some(Outcome::Quit));
        assert_eq!(
            a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Outcome::Quit)
        );
    }

    #[test]
    fn ctrl_y_says_why_a_session_has_no_resume_command() {
        let mut a = app("");
        a.idx.rows[0].session.as_mut().unwrap().cwd = Some("/tmp/a\u{1b}[2Jb".into());
        a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        let msg = a.msg.clone().unwrap();
        assert!(msg.contains("control characters"), "{msg}");
    }

    #[test]
    fn enter_on_a_live_session_asks_first() {
        let mut a = app("");
        a.idx.rows[1].session.as_mut().unwrap().live_status = Some("idle".into());
        let id = a.idx.rows[1].id.clone();
        a.key(key(KeyCode::Down));
        assert_eq!(a.current().unwrap().id, id);
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert!(a.msg.as_deref().unwrap().contains("another terminal"));
        // Esc cancels the confirm, a second Esc quits
        assert!(a.key(key(KeyCode::Esc)).is_none());
        assert!(a.msg.is_none());
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert_eq!(a.key(key(KeyCode::Enter)), Some(Outcome::Pick(id.clone())));
        a.key(key(KeyCode::Esc));
        // moving away and back clears the pending confirm
        assert!(a.key(key(KeyCode::Enter)).is_none());
        a.key(key(KeyCode::Up));
        a.key(key(KeyCode::Down));
        assert!(a.msg.is_none());
        assert!(a.key(key(KeyCode::Enter)).is_none());
        // typing clears it too
        a.key(key(KeyCode::Char('x')));
        a.key(key(KeyCode::Backspace));
        a.key(key(KeyCode::Down));
        assert_eq!(a.current().unwrap().id, id);
        assert!(a.msg.is_none());
        assert!(a.key(key(KeyCode::Enter)).is_none());
    }

    /// The picker on its idea, whose folder is `sub` inside a root eye may create folders in.
    fn idea_app(sub: &str) -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        let mut a = app("zephyr");
        let dir = tmp.path().join(sub).to_string_lossy().into_owned();
        let row = &mut a.idx.rows[a.results[0]];
        row.folder_state = Some(crate::folder::classify(Some(tmp.path()), &dir));
        row.folder = Some(dir.clone());
        row.idea.as_mut().unwrap().folder = Some(dir);
        (tmp, a)
    }

    /// The idea row's folder asked about again, as loading the rows would, with `root` as the
    /// folder eye may create folders inside.
    fn classify_again(a: &mut App, root: &std::path::Path) {
        let row = &mut a.idx.rows[a.results[0]];
        let dir = row.folder.clone().unwrap();
        row.folder_state = Some(crate::folder::classify(Some(root), &dir));
    }

    #[test]
    fn an_idea_with_a_missing_folder_asks_before_it_starts() {
        let (tmp, mut a) = idea_app("new");
        let id = a.current().unwrap().id.clone();
        assert!(a.key(key(KeyCode::Enter)).is_none());
        let m = a.msg.clone().unwrap();
        assert!(
            m.contains("does not exist") && m.contains("create it"),
            "{m}"
        );
        // Esc cancels, and the next Enter asks again
        assert!(a.key(key(KeyCode::Esc)).is_none());
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert_eq!(a.key(key(KeyCode::Enter)), Some(Outcome::Pick(id)));
        // the picker itself creates nothing: the launch does, once it is told to
        assert!(!tmp.path().join("new").exists());
    }

    #[test]
    fn an_idea_in_a_folder_that_exists_starts_at_once() {
        let (_tmp, mut a) = idea_app("");
        let id = a.current().unwrap().id.clone();
        assert_eq!(a.key(key(KeyCode::Enter)), Some(Outcome::Pick(id)));
    }

    #[test]
    fn an_idea_whose_folder_cannot_be_made_says_why_and_does_not_start() {
        let (tmp, mut a) = idea_app("new");
        classify_again(&mut a, &tmp.path().join("elsewhere"));
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert!(a
            .msg
            .as_deref()
            .unwrap()
            .contains("inside your home folder"));
        let (tmp, mut a) = idea_app("file");
        std::fs::write(tmp.path().join("file"), "x").unwrap();
        classify_again(&mut a, tmp.path());
        assert!(a.key(key(KeyCode::Enter)).is_none());
        assert_eq!(a.msg.as_deref(), Some("Not a folder"));
    }

    #[test]
    fn the_list_and_the_preview_mark_an_idea_with_a_missing_folder() {
        let (_tmp, mut a) = idea_app("new");
        let t = render_text(&mut a, 120, 24).unwrap();
        assert!(t.contains("+auth"), "{t}");
        assert!(t.contains("Start will create it"), "{t}");
        let (_tmp, mut a) = idea_app("");
        let t = render_text(&mut a, 120, 24).unwrap();
        assert!(!t.contains("+auth") && !t.contains("does not exist"), "{t}");
    }

    #[test]
    fn ctrl_x_hides_and_ctrl_a_finds_it_again() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let mut a = app("");
        let id = a.current().unwrap().id.clone();
        a.db.conn
            .execute("INSERT INTO session(id) VALUES(?1)", [&id])
            .unwrap();
        let n = a.results.len();
        a.key(ctrl('x'));
        assert!(a.msg.as_deref().unwrap().starts_with("hidden"));
        assert_eq!(a.results.len(), n - 1);
        assert_ne!(a.current().unwrap().id, id);
        assert_eq!(a.db.session_by_id(&id).unwrap().unwrap().hidden, Some(true));
        a.key(ctrl('a'));
        while a.current().unwrap().id != id {
            a.key(key(KeyCode::Down));
        }
        a.key(ctrl('x'));
        assert!(a.msg.as_deref().unwrap().starts_with("shown again"));
        assert_eq!(
            a.db.session_by_id(&id).unwrap().unwrap().hidden,
            Some(false)
        );
    }

    #[test]
    fn ctrl_t_shows_the_conversation_newest_at_the_bottom_and_scrolls() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let (_tmp, mut a) = talk_app(30);
        let before = render_text(&mut a, 120, 20).unwrap();
        assert!(before.contains("Next (you)") && !before.contains("Conversation"));
        a.key(ctrl('t'));
        let t = render_text(&mut a, 120, 20).unwrap();
        assert!(t.contains("Conversation · 60 messages"), "{t}");
        assert!(t.contains("answer 29") && !t.contains("question 0"), "{t}");
        assert!(t.contains("1 tool calls"));
        // the end is at the bottom of the pane
        let last = t.lines().rev().find(|l| l.contains("answer 29")).unwrap();
        assert!(t.lines().position(|l| l == last).unwrap() >= 17, "{t}");
        let shift = |c| KeyEvent::new(c, KeyModifiers::SHIFT);
        for _ in 0..4 {
            a.key(shift(KeyCode::Up));
        }
        let up = render_text(&mut a, 120, 20).unwrap();
        assert!(!up.contains("answer 29"), "{up}");
        assert_eq!(
            a.current().unwrap().column,
            Column::NeedsYou,
            "row did not move"
        );
        for _ in 0..4 {
            a.key(shift(KeyCode::Down));
        }
        assert!(render_text(&mut a, 120, 20).unwrap().contains("answer 29"));
        // scrolling stops at the oldest message
        for _ in 0..500 {
            a.key(shift(KeyCode::Up));
        }
        assert!(render_text(&mut a, 120, 20).unwrap().contains("question 0"));
        // back to the summary
        a.key(ctrl('t'));
        assert!(render_text(&mut a, 120, 20).unwrap().contains("Next (you)"));
    }

    #[test]
    fn ctrl_f_forks_without_the_open_elsewhere_confirm() {
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        let mut a = app("");
        a.idx.rows[0].session.as_mut().unwrap().live_status = Some("busy".into());
        let id = a.current().unwrap().id.clone();
        assert_eq!(a.key(ctrl('f')), Some(Outcome::Fork(id)));
        // an idea cannot be forked
        for c in "hyd".chars() {
            a.key(key(KeyCode::Char(c)));
        }
        assert!(a.key(ctrl('f')).is_none());
        assert!(a.msg.as_deref().unwrap().contains("cannot be forked"));
        // the footer names both keys, also in a narrow terminal
        for w in [60, 80, 120, 160] {
            let t = render_text(&mut app(""), w, 24).unwrap();
            let foot = t.lines().rev().find(|l| !l.is_empty()).unwrap();
            assert!(
                foot.contains("^F fork") || foot.contains("Ctrl-F fork"),
                "{foot}"
            );
            assert!(foot.contains("talk"), "{foot}");
        }
    }
}
