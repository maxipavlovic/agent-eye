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
