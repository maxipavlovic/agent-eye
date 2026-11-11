use crate::activity::{self, DIALOG};
use crate::model::{AiStatus, Column, Session};

const SEVEN_DAYS_MS: i64 = 7 * 24 * 3600 * 1000;

/// Everything the column rules look at.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    pub archived: bool,
    /// A "check on this" reminder has come due and the session is not touched since.
    pub remind_due: bool,
    pub column_manual: Option<Column>,
    pub column_manual_at: Option<i64>,
    pub last_activity_at: i64,
    pub live_status: Option<&'a str>,
    /// Why the live status says `waiting` (`permission prompt`).
    pub live_wait: Option<&'a str>,
    /// The session has a background task or a loop going.
    pub has_background: bool,
    /// A hook event says the session waits on the user.
    pub hook_waiting: bool,
    pub last_assistant_text: Option<&'a str>,
    pub ai_status: Option<AiStatus>,
    pub kind: &'a str,
    pub prompt_count: i64,
    pub has_outputs: bool,
    pub now_ms: i64,
}

impl<'a> Input<'a> {
    /// The input for a session; `has_outputs` says it has any output.
    pub fn from_session(s: &'a Session, has_outputs: bool, now_ms: i64) -> Input<'a> {
        Input {
            archived: s.archived,
            remind_due: crate::remind::is_due(s.remind_at, s.last_activity_at.unwrap_or(0), now_ms),
            column_manual: s.column_manual.as_deref().and_then(Column::parse),
            column_manual_at: s.column_manual_at,
            last_activity_at: s.last_activity_at.unwrap_or(0),
            live_status: s.live_status.as_deref(),
            live_wait: s.live_wait.as_deref(),
            has_background: activity::has_work(s, now_ms),
            hook_waiting: s.waiting_since.is_some(),
            last_assistant_text: s.last_assistant_text.as_deref(),
            ai_status: s.ai_status.as_deref().and_then(AiStatus::parse),
            kind: s.kind.as_deref().unwrap_or("interactive"),
            prompt_count: s.prompt_count,
            has_outputs,
            now_ms,
        }
    }
}

/// The column a session belongs in; the first matching rule wins.
pub fn compute(i: &Input) -> Column {
    if i.archived {
        return Column::Archive;
    }
    // it comes back whatever the session was doing or wherever you put it
    if i.remind_due {
        return Column::NeedsYou;
    }
    if let Some(c) = i.column_manual {
        if i.column_manual_at.unwrap_or(0) >= i.last_activity_at {
            return c;
        }
    }
    if i.live_status == Some("busy") {
        return Column::Running;
    }
    if let Some(ls) = i.live_status {
        let asks = i
            .last_assistant_text
            .map(|t| t.trim_end().ends_with('?'))
            .unwrap_or(false);
        if i.hook_waiting || (ls == "idle" && asks) {
            return Column::NeedsYou;
        }
        let dialog = i.live_wait.is_some_and(|w| w.eq_ignore_ascii_case(DIALOG));
        let live_waiting = ls == "waiting" && !dialog;
        if live_waiting && i.live_wait.is_some() {
            return Column::NeedsYou;
        }
        if i.has_background && !asks && !live_waiting {
            return Column::Running;
        }
    }
    match i.ai_status {
        Some(AiStatus::NeedsYou) if i.now_ms - i.last_activity_at <= SEVEN_DAYS_MS => {
            Column::NeedsYou
        }
        Some(AiStatus::WaitingOnOthers) => Column::Waiting,
        Some(AiStatus::Done) => Column::Done,
        Some(_) => Column::Paused,
        None => {
            // a session that is open now is work in progress, however little it has so far
            let trivial = i.prompt_count <= 1 && !i.has_outputs && i.live_status.is_none();
            if i.kind == "history_only" || trivial {
                Column::Archive
            } else {
                Column::Paused
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Input<'static> {
        Input {
            archived: false,
            remind_due: false,
            column_manual: None,
            column_manual_at: None,
            last_activity_at: 1_000,
            live_status: None,
            live_wait: None,
            has_background: false,
            hook_waiting: false,
            last_assistant_text: None,
            ai_status: None,
            kind: "interactive",
            prompt_count: 5,
            has_outputs: false,
            now_ms: 2_000,
        }
    }

    #[test]
    fn archived_beats_everything() {
        let mut i = base();
        i.archived = true;
        i.column_manual = Some(Column::Done);
        i.column_manual_at = Some(5_000);
        i.live_status = Some("busy");
        assert_eq!(compute(&i), Column::Archive);
    }

    #[test]
    fn a_due_reminder_beats_the_manual_column_and_the_rules_but_not_the_archive() {
        let mut i = base();
        i.remind_due = true;
        i.column_manual = Some(Column::Waiting);
        i.column_manual_at = Some(1_500);
        assert_eq!(compute(&i), Column::NeedsYou);
        i.live_status = Some("busy");
        i.ai_status = Some(AiStatus::Done);
        assert_eq!(compute(&i), Column::NeedsYou);
        i.archived = true;
        assert_eq!(compute(&i), Column::Archive);
    }

    #[test]
    fn a_session_with_a_reminder_that_is_not_due_stays_where_the_rules_put_it() {
        let mut s = Session {
            remind_at: Some(5_000),
            last_activity_at: Some(1_000),
            column_manual: Some("waiting".into()),
            column_manual_at: Some(1_000),
            ..Default::default()
        };
        let col = |s: &Session, now| compute(&Input::from_session(s, true, now));
        assert_eq!(col(&s, 2_000), Column::Waiting, "not yet");
        assert_eq!(col(&s, 5_000), Column::NeedsYou, "due");
        s.last_activity_at = Some(5_001);
        assert_eq!(
            col(&s, 6_000),
            Column::Paused,
            "resumed: the reminder is answered"
        );
    }

    #[test]
    fn manual_until_new_activity() {
        let mut i = base();
        i.column_manual = Some(Column::Done);
        i.column_manual_at = Some(1_000);
        assert_eq!(compute(&i), Column::Done);
        i.last_activity_at = 1_500;
        assert_eq!(compute(&i), Column::Paused);
    }

    #[test]
    fn live_rules() {
        let mut i = base();
        i.live_status = Some("busy");
        assert_eq!(compute(&i), Column::Running);
        i.live_status = Some("idle");
        i.last_assistant_text = Some("Shall I go on?  \n");
        assert_eq!(compute(&i), Column::NeedsYou);
        i.last_assistant_text = Some("Done.");
        assert_eq!(compute(&i), Column::Paused);
        i.hook_waiting = true;
        assert_eq!(compute(&i), Column::NeedsYou);
        // not open: a trailing question alone does not count
        i.live_status = None;
        i.hook_waiting = false;
        i.last_assistant_text = Some("ok?");
        assert_eq!(compute(&i), Column::Paused);
    }

    #[test]
    fn a_session_that_waits_on_a_prompt_needs_you() {
        let mut i = base();
        i.live_status = Some("waiting");
        i.live_wait = Some("permission prompt");
        assert_eq!(compute(&i), Column::NeedsYou);
        i.live_wait = Some("input needed");
        assert_eq!(compute(&i), Column::NeedsYou);
        i.ai_status = Some(AiStatus::Done);
        assert_eq!(
            compute(&i),
            Column::NeedsYou,
            "a live ask beats the summary"
        );
        i.ai_status = None;
        i.live_wait = Some("dialog open");
        assert_eq!(
            compute(&i),
            Column::Paused,
            "a dialog the user opened is no ask"
        );
        i.live_wait = None;
        assert_eq!(compute(&i), Column::Paused, "no reason given: as before");
    }

    #[test]
    fn background_work_keeps_an_open_session_in_running() {
        let mut i = base();
        i.has_background = true;
        assert_eq!(compute(&i), Column::Paused, "a closed session never runs");
        i.live_status = Some("idle");
        assert_eq!(compute(&i), Column::Running);
        i.last_assistant_text = Some("Waiting for the build.");
        assert_eq!(compute(&i), Column::Running);
        i.live_status = Some("waiting");
        i.live_wait = Some("dialog open");
        assert_eq!(compute(&i), Column::Running, "a dialog is not an ask");
        i.has_background = false;
        assert_eq!(compute(&i), Column::Paused);
    }

    #[test]
    fn background_work_does_not_hide_that_you_are_asked() {
        let mut i = base();
        i.has_background = true;
        i.live_status = Some("idle");
        i.last_assistant_text = Some("Shall I deploy it?");
        assert_eq!(compute(&i), Column::NeedsYou);
        i.last_assistant_text = Some("Done.");
        i.hook_waiting = true;
        assert_eq!(compute(&i), Column::NeedsYou);
        i.hook_waiting = false;
        i.live_status = Some("waiting");
        i.live_wait = Some("permission prompt");
        assert_eq!(compute(&i), Column::NeedsYou);
        i.live_wait = None;
        assert_eq!(
            compute(&i),
            Column::Paused,
            "waiting for unknown reasons stays put"
        );
    }

    #[test]
    fn background_work_does_not_beat_a_manual_column_or_the_archive() {
        let mut i = base();
        i.live_status = Some("idle");
        i.has_background = true;
        i.column_manual = Some(Column::Done);
        i.column_manual_at = Some(1_000);
        assert_eq!(compute(&i), Column::Done);
        i.archived = true;
        assert_eq!(compute(&i), Column::Archive);
    }

    #[test]
    fn ai_status_rules() {
        let mut i = base();
        i.ai_status = Some(AiStatus::NeedsYou);
        assert_eq!(compute(&i), Column::NeedsYou);
        i.now_ms = 1_000 + SEVEN_DAYS_MS + 1;
        assert_eq!(compute(&i), Column::Paused);
        i.ai_status = Some(AiStatus::WaitingOnOthers);
        assert_eq!(compute(&i), Column::Waiting);
        i.ai_status = Some(AiStatus::Done);
        assert_eq!(compute(&i), Column::Done);
        i.ai_status = Some(AiStatus::Paused);
        assert_eq!(compute(&i), Column::Paused);
    }

    #[test]
    fn no_ai_status() {
        let mut i = base();
        assert_eq!(compute(&i), Column::Paused);
        i.prompt_count = 1;
        assert_eq!(compute(&i), Column::Archive);
        i.has_outputs = true;
        assert_eq!(compute(&i), Column::Paused);
        i.kind = "history_only";
        assert_eq!(compute(&i), Column::Archive);
    }

    #[test]
    fn an_open_session_is_never_archived_for_having_little() {
        let mut i = base();
        i.prompt_count = 1;
        i.live_status = Some("idle");
        assert_eq!(compute(&i), Column::Paused);
        i.live_status = Some("waiting");
        assert_eq!(compute(&i), Column::Paused);
        i.live_status = Some("busy");
        assert_eq!(compute(&i), Column::Running);
        i.live_status = None;
        assert_eq!(compute(&i), Column::Archive, "closed, it is trivial again");
        i.archived = true;
        i.live_status = Some("idle");
        assert_eq!(
            compute(&i),
            Column::Archive,
            "archiving it by hand still works"
        );
    }
}
