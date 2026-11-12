//! The reminder on the selected row: the one-line input (Ctrl-R) and how a row shows its reminder.

use super::state::App;
use super::Outcome;
use crate::columns;
use crate::ideas;
use crate::model::Column;
use crate::pick::{Kind, Row};
use crate::remind::{self, notices, Target};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// What the footer says while the input is open.
pub(super) const HINT: &str = "tomorrow  mon..sun  30m  4h  3d  1w  2026-11-20 14:00   add -- and a note   empty clears   Esc cancels";

/// The input line: the text so far, and what was wrong with the last try.
#[derive(Debug, Default)]
pub(super) struct Input {
    pub text: String,
    pub error: Option<String>,
    /// Whether a due reminder shows a desktop notice; None when the picker has no eye home to ask.
    pub notify: Option<bool>,
    /// Can this machine show a notice at all?
    pub can_notify: bool,
}

impl Input {
    /// The footer's words for the notice setting, and the key that flips it.
    pub(super) fn notice_state(&self) -> Option<String> {
        let on = self.notify?;
        Some(match (on, self.can_notify) {
            (true, true) => "notices on (Tab: turn off)".into(),
            (true, false) => "notices on, but no program shows them here (Tab: turn off)".into(),
            (false, true) => "notices off (Tab: turn on)".into(),
            (false, false) => "notices off, and no program shows them here".into(),
        })
    }
}

/// `tomorrow -- Ana's reply` as the time words and the note.
fn split_note(text: &str) -> (&str, Option<&str>) {
    match text.split_once(" -- ") {
        Some((when, note)) => (when.trim(), Some(note.trim())),
        None => (text.trim(), None),
    }
}

/// The chip a row carries: `⏰ due`, or the time when it is within a few days; nothing further out.
pub(super) fn chip(r: &Row, now: i64) -> Option<String> {
    let rem = r.remind.as_ref()?;
    if rem.due {
        return Some("⏰ due".into());
    }
    (rem.at - now <= remind::SOON_DAYS * 86_400_000)
        .then(|| format!("⏰ {}", remind::label(rem.at, now)))
}

impl App {
    /// Open the input for the selected row.
    pub(super) fn start_remind(&mut self) {
        if self.current().is_none() {
            self.msg = Some("nothing selected".into());
            return;
        }
        let notify = self
            .paths
            .as_ref()
            .map(|p| notices::is_on(p, crate::config::RemindCfg::default().notify));
        self.remind = Some(Input {
            notify,
            can_notify: notify.is_some() && notices::available(),
            ..Input::default()
        });
    }

    /// Tab in the input: switch the desktop notice for due reminders on or off.
    fn toggle_notices(&mut self) {
        let (Some(paths), Some(input)) = (self.paths.as_ref(), self.remind.as_mut()) else {
            return;
        };
        let Some(on) = input.notify else {
            return;
        };
        if !on && !input.can_notify {
            return;
        }
        match notices::set(paths, !on) {
            Ok(()) => input.notify = Some(!on),
            Err(e) => input.error = Some(format!("could not save: {e:#}")),
        }
    }

    pub(super) fn remind_key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let input = self.remind.as_mut()?;
        input.error = None;
        match k.code {
            KeyCode::Char('c') if ctrl => return Some(Outcome::Quit),
            KeyCode::Esc => self.remind = None,
            KeyCode::Enter => self.apply_remind(),
            KeyCode::Tab => self.toggle_notices(),
            KeyCode::Char('u') if ctrl => input.text.clear(),
            KeyCode::Backspace => {
                input.text.pop();
            }
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                input.text.push(c)
            }
            _ => {}
        }
        None
    }

    /// Set the reminder from the text, or clear it when the text is empty. A text that cannot be
    /// read leaves the input open with the reason.
    fn apply_remind(&mut self) {
        let Some(text) = self.remind.as_ref().map(|i| i.text.trim().to_string()) else {
            return;
        };
        let Some(id) = self.current().map(|r| r.id.clone()) else {
            self.remind = None;
            return;
        };
        let said = if text.is_empty() {
            self.save_remind(&id, None)
        } else {
            let (when, note) = split_note(&text);
            match remind::parse_local(when, self.now) {
                Ok(at) => self.save_remind(&id, Some((at, note))),
                Err(e) => {
                    if let Some(i) = self.remind.as_mut() {
                        i.error = Some(format!("{e:#}"));
                    }
                    return;
                }
            }
        };
        self.remind = None;
        self.reselect(&id);
        self.msg = Some(said.unwrap_or_else(|e| format!("could not save: {e:#}")));
    }

    /// Write the reminder (None clears it), read the row again, and say what happened.
    fn save_remind(&mut self, id: &str, set: Option<(i64, Option<&str>)>) -> Result<String> {
        let target = Target::resolve(&self.db, id)?;
        let said = match set {
            Some((at, note)) => {
                remind::set(&self.db, &target, at, note)?;
                format!("reminder set: {}", remind::label(at, self.now))
            }
            None => {
                remind::clear(&self.db, &target)?;
                "reminder cleared".into()
            }
        };
        self.reread(id)?;
        Ok(said)
    }

    /// Read a row's session or idea again, with its column and reminder.
    fn reread(&mut self, id: &str) -> Result<()> {
        let now = self.now;
        let at = self
            .idx
            .rows
            .iter()
            .position(|r| r.id == id)
            .context("the row is gone")?;
        let row = &mut self.idx.rows[at];
        match row.kind {
            Kind::Session => {
                let s = self.db.session_by_id(id)?.context("no such session")?;
                let has_outputs = !self.db.outputs_for(id)?.is_empty();
                row.column = columns::compute(&columns::Input::from_session(&s, has_outputs, now));
                row.remind = remind::of_session(&s, now);
                row.session = Some(s);
            }
            Kind::Idea => {
                let n: i64 = id.trim_start_matches("idea:").parse()?;
                let i = ideas::get(&self.db, n)?.context("no such idea")?;
                row.remind = remind::of_idea(&i, now);
                row.column = if row.remind.as_ref().is_some_and(|r| r.due) {
                    Column::NeedsYou
                } else {
                    Column::Ideas
                };
                row.idea = Some(i);
            }
        }
        Ok(())
    }

    /// Rank the rows again and keep the selection on the row `id`.
    fn reselect(&mut self, id: &str) {
        self.research();
        if let Some(p) = self.results.iter().position(|&i| self.idx.rows[i].id == id) {
            self.sel = p;
        }
        self.refresh_detail();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ideas::NewIdea;
    use crate::tui::render_text;
    use crate::tui::test_support::*;
    use crossterm::event::KeyCode;

    const FIRST: &str = "aaaaaaaa-0000-4000-8000-000000000001";

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(a: &mut App, text: &str) {
        for c in text.chars() {
            a.key(key(KeyCode::Char(c)));
        }
    }

    /// The picker with the first session in the database, so a reminder can be saved on it.
    fn saved_app() -> App {
        let a = app("");
        a.db.conn
            .execute(
                "INSERT INTO session(id,title,last_activity_at,prompt_count) VALUES(?1,'Catalog',1,5)",
                [FIRST],
            )
            .unwrap();
        a
    }

    fn select(a: &mut App, id: &str) {
        a.sel = a
            .results
            .iter()
            .position(|&i| a.idx.rows[i].id == id)
            .unwrap();
        a.refresh_detail();
    }

    fn row<'a>(a: &'a App, id: &str) -> &'a Row {
        a.idx.rows.iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn ctrl_r_sets_a_reminder_with_a_note_and_the_row_and_preview_show_it() {
        let mut a = saved_app();
        select(&mut a, FIRST);
        a.key(ctrl('r'));
        assert!(a.remind.is_some());
        type_text(&mut a, "tomorrow -- Ana's reply");
        let foot = render_text(&mut a, 160, 24).unwrap();
        assert!(foot.contains("remind › tomorrow -- Ana's reply"), "{foot}");
        assert!(
            a.key(key(KeyCode::Enter)).is_none(),
            "Enter sets it, it does not resume"
        );
        assert!(a.remind.is_none());
        assert_eq!(a.msg.as_deref(), Some("reminder set: tomorrow 09:00"));
        let s = a.db.session_by_id(FIRST).unwrap().unwrap();
        assert_eq!(s.remind_note.as_deref(), Some("Ana's reply"));
        assert_eq!(
            row(&a, FIRST).remind.as_ref().unwrap().at,
            s.remind_at.unwrap()
        );
        assert_eq!(
            a.current().unwrap().id,
            FIRST,
            "the selection stays on the row"
        );
        let t = render_text(&mut a, 120, 24).unwrap();
        assert!(
            t.contains("⏰ tomorrow 09:00 "),
            "the row has the chip:\n{t}"
        );
        assert!(
            t.contains("⏰ tomorrow 09:00 · Ana's reply"),
            "the preview has it:\n{t}"
        );
    }

    #[test]
    fn a_reminder_further_out_than_a_few_days_is_in_the_preview_only() {
        let mut a = saved_app();
        select(&mut a, FIRST);
        a.key(ctrl('r'));
        type_text(&mut a, "2w");
        a.key(key(KeyCode::Enter));
        let r = row(&a, FIRST);
        assert!(chip(r, a.now).is_none());
        let t = render_text(&mut a, 120, 24).unwrap();
        assert_eq!(t.matches('⏰').count(), 1, "only the preview:\n{t}");
    }

    #[test]
    fn a_text_that_cannot_be_read_keeps_the_input_open_and_says_why() {
        let mut a = saved_app();
        select(&mut a, FIRST);
        a.key(ctrl('r'));
        type_text(&mut a, "soon");
        a.key(key(KeyCode::Enter));
        let input = a.remind.as_ref().unwrap();
        assert_eq!(input.text, "soon");
        assert!(input.error.as_deref().unwrap().contains("tomorrow"));
        let foot = render_text(&mut a, 200, 24).unwrap();
        assert!(foot.contains("cannot read 'soon'"), "{foot}");
        // typing again clears the complaint; Esc leaves without saving
        type_text(&mut a, "x");
        assert!(a.remind.as_ref().unwrap().error.is_none());
        assert!(
            a.key(key(KeyCode::Esc)).is_none(),
            "Esc closes the input, it does not quit"
        );
        assert!(a.remind.is_none());
        assert!(a
            .db
            .session_by_id(FIRST)
            .unwrap()
            .unwrap()
            .remind_at
            .is_none());
    }

    #[test]
    fn an_empty_line_clears_the_reminder() {
        let mut a = saved_app();
        select(&mut a, FIRST);
        a.key(ctrl('r'));
        type_text(&mut a, "1d");
        a.key(key(KeyCode::Enter));
        assert!(row(&a, FIRST).remind.is_some());
        a.key(ctrl('r'));
        a.key(key(KeyCode::Enter));
        assert_eq!(a.msg.as_deref(), Some("reminder cleared"));
        assert!(row(&a, FIRST).remind.is_none());
        assert!(a
            .db
            .session_by_id(FIRST)
            .unwrap()
            .unwrap()
            .remind_at
            .is_none());
    }

    #[test]
    fn a_due_reminder_puts_the_row_in_needs_you_and_says_why() {
        let mut a = saved_app();
        let id = "cccccccc-0000-4000-8000-000000000003";
        a.db.conn
            .execute(
                "INSERT INTO session(id,title,last_activity_at,prompt_count) VALUES(?1,'Waiting',1,5)",
                [id],
            )
            .unwrap();
        a.db.conn
            .execute(
                "UPDATE session SET remind_at=?2, remind_note='Ana''s reply' WHERE id=?1",
                rusqlite::params![id, a.now - 1000],
            )
            .unwrap();
        a.reread(id).unwrap();
        a.reselect(id);
        assert_eq!(row(&a, id).column, Column::NeedsYou);
        assert_eq!(
            a.results.first(),
            a.idx.rows.iter().position(|r| r.id == id).as_ref()
        );
        let t = render_text(&mut a, 120, 24).unwrap();
        assert!(t.contains("⏰ due"), "{t}");
        assert!(t.contains("⏰ check on: Ana's reply"), "{t}");
    }

    #[test]
    fn an_idea_can_be_reminded_too() {
        let mut a = app("");
        let idea = crate::ideas::add(
            &a.db,
            &NewIdea {
                title: "Try the Zephyr demo again",
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(idea.id, 1);
        select(&mut a, "idea:1");
        a.key(ctrl('r'));
        type_text(&mut a, "mon -- before the demo");
        a.key(key(KeyCode::Enter));
        assert!(a.msg.as_deref().unwrap().starts_with("reminder set"));
        let r = row(&a, "idea:1");
        assert_eq!(
            r.remind.as_ref().unwrap().note.as_deref(),
            Some("before the demo")
        );
        assert_eq!(r.column, Column::Ideas, "not due yet");
        assert_eq!(a.current().unwrap().id, "idea:1");
    }

    fn app_at_home() -> (tempfile::TempDir, App) {
        let t = tempfile::tempdir().unwrap();
        let mut a = saved_app();
        a.paths = Some(crate::paths::Paths {
            home: t.path().join("home"),
        });
        select(&mut a, FIRST);
        (t, a)
    }

    #[test]
    fn tab_in_the_input_switches_desktop_notices_and_the_footer_says_which() {
        let (_t, mut a) = app_at_home();
        a.key(ctrl('r'));
        a.remind.as_mut().unwrap().can_notify = true;
        let foot = render_text(&mut a, 200, 24).unwrap();
        assert!(foot.contains("notices off (Tab: turn on)"), "{foot}");
        a.key(key(KeyCode::Tab));
        let foot = render_text(&mut a, 200, 24).unwrap();
        assert!(foot.contains("notices on"), "{foot}");
        assert!(
            a.remind.as_ref().unwrap().text.is_empty(),
            "Tab types nothing"
        );
        let paths = a.paths.clone().unwrap();
        assert!(notices::is_on(&paths, false), "written to config.toml");
        a.key(key(KeyCode::Esc));
        a.key(ctrl('r'));
        assert_eq!(a.remind.as_ref().unwrap().notify, Some(true), "it stays");
        a.key(key(KeyCode::Tab));
        assert!(!notices::is_on(&paths, true));
    }

    #[test]
    fn tab_does_nothing_when_no_program_can_show_a_notice_and_notices_are_off() {
        let (_t, mut a) = app_at_home();
        a.key(ctrl('r'));
        a.remind.as_mut().unwrap().can_notify = false;
        a.key(key(KeyCode::Tab));
        assert_eq!(a.remind.as_ref().unwrap().notify, Some(false));
        let paths = a.paths.clone().unwrap();
        assert!(!paths.config_path().exists(), "nothing was written");
        assert!(!notices::is_on(&paths, false));
    }

    #[test]
    fn without_an_eye_home_the_input_has_no_notice_line_and_tab_does_nothing() {
        let mut a = saved_app();
        select(&mut a, FIRST);
        a.paths = None;
        a.key(ctrl('r'));
        a.key(key(KeyCode::Tab));
        let foot = render_text(&mut a, 200, 24).unwrap();
        assert!(!foot.contains("notices"), "{foot}");
    }

    #[test]
    fn a_row_that_is_not_in_the_database_says_it_could_not_save() {
        let mut a = app("");
        a.key(ctrl('r'));
        type_text(&mut a, "1d");
        a.key(key(KeyCode::Enter));
        assert!(
            a.msg.as_deref().unwrap().starts_with("could not save"),
            "{:?}",
            a.msg
        );
    }

    #[test]
    fn the_footer_names_the_key_where_there_is_room() {
        for w in [120, 160] {
            let t = render_text(&mut app(""), w, 24).unwrap();
            let foot = t.lines().rev().find(|l| !l.is_empty()).unwrap();
            assert!(
                foot.contains("^R remind") || foot.contains("Ctrl-R remind"),
                "{foot}"
            );
        }
    }

    #[test]
    fn the_note_comes_after_two_dashes() {
        assert_eq!(split_note("tomorrow"), ("tomorrow", None));
        assert_eq!(split_note("3d -- the reply"), ("3d", Some("the reply")));
        assert_eq!(
            split_note("2026-11-20 14:00 -- a -- b"),
            ("2026-11-20 14:00", Some("a -- b"))
        );
    }
}
