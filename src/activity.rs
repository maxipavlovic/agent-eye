//! What an open session does on its own: background tasks it started and loops it runs. An adapter
//! reports them, the scanner stores them, and the column rules, cards, picker and CLI read them
//! through the functions here, which also decide what still counts.

use crate::model::Session;
use crate::util::{fmt_clock, head_chars, printable};
use serde::{Deserialize, Serialize};

/// A background task whose end was never seen is dropped after this long (a closed session, a
/// killed shell and a lost notice all leave one behind).
pub const TASK_MAX_AGE_MS: i64 = 12 * 3600 * 1000;
/// A recurring loop made by a cron-like schedule expires on its own after this long.
pub const INTERVAL_MAX_AGE_MS: i64 = 7 * 24 * 3600 * 1000;
/// A self-paced loop that has not scheduled its next wake-up this long after the last one is over.
pub const WAKE_GRACE_MS: i64 = 15 * 60 * 1000;
/// Longest label of a task or a loop prompt, in characters.
pub const LABEL_MAX: usize = 80;

/// A background task, the `kind` of which is `shell`, `agent` or `monitor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BgTask {
    pub id: String,
    pub kind: String,
    /// The command or description, one line.
    pub label: String,
    pub started_at: i64,
}

/// A loop, the `kind` of which is `interval`, `self_paced` or `goal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopInfo {
    pub kind: String,
    /// "every 10 min", "daily at 09:00", "goal: <objective>"; empty for a self-paced loop.
    pub schedule: String,
    /// The next run, when the agent says it.
    pub next_at: Option<i64>,
    /// What the loop does each time, shortened.
    pub prompt: String,
    pub started_at: i64,
    /// The cron expression of an interval loop, to work out its next run.
    #[serde(default)]
    pub cron: Option<String>,
}

/// What a transcript says about a session's tasks and loops.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signals {
    #[serde(default)]
    pub tasks: Vec<BgTask>,
    #[serde(default)]
    pub loops: Vec<LoopInfo>,
}

impl Signals {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.loops.is_empty()
    }
}

/// The text a database column keeps for a value; None for an empty one.
pub fn to_column<T: Serialize + Default + PartialEq>(value: &T) -> Option<String> {
    if *value == T::default() {
        None
    } else {
        serde_json::to_string(value).ok()
    }
}

/// A value read back from a database column; anything unreadable is the empty value.
pub fn from_column<T: for<'de> Deserialize<'de> + Default>(text: Option<&str>) -> T {
    text.and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_default()
}

/// One line of any text, shortened to [`LABEL_MAX`] characters, with no control characters (a
/// command can hold an escape sequence, and this is shown in terminals).
pub fn one_line(text: &str) -> String {
    let text = printable(text);
    head_chars(
        &text.split_whitespace().collect::<Vec<_>>().join(" "),
        LABEL_MAX,
    )
}

impl LoopInfo {
    /// When the loop runs next: what the agent said if that is still ahead, else worked out for a
    /// plain every-N-minutes expression.
    pub fn next_run(&self, now: i64) -> Option<i64> {
        if let Some(at) = self.next_at.filter(|at| *at >= now) {
            return Some(at);
        }
        let minutes = self.cron.as_deref()?.strip_prefix("*/")?;
        let (n, rest) = minutes.split_once(' ')?;
        let n: i64 = n.parse().ok().filter(|n| *n > 0 && 60 % n == 0)?;
        (rest.trim() == "* * * *").then(|| ((now / 60_000) / n + 1) * n * 60_000)
    }

    fn live(&self, now: i64) -> bool {
        match self.kind.as_str() {
            "interval" => now - self.started_at <= INTERVAL_MAX_AGE_MS,
            "self_paced" => self.next_at.is_some_and(|at| now <= at + WAKE_GRACE_MS),
            _ => true,
        }
    }
}

/// The background tasks of an open session that are not too old to believe.
pub fn tasks(s: &Session, now: i64) -> Vec<&BgTask> {
    if s.live_status.is_none() {
        return vec![];
    }
    s.signals
        .tasks
        .iter()
        .filter(|t| now - t.started_at <= TASK_MAX_AGE_MS)
        .collect()
}

/// The loops of an open session that are still running.
pub fn loops(s: &Session, now: i64) -> Vec<&LoopInfo> {
    if s.live_status.is_none() {
        return vec![];
    }
    s.signals
        .loops
        .iter()
        .chain(&s.live_loops)
        .filter(|l| l.live(now))
        .collect()
}

/// Does the session have a task or a loop going?
pub fn has_work(s: &Session, now: i64) -> bool {
    !tasks(s, now).is_empty() || !loops(s, now).is_empty()
}

/// "<1 min", "4 min", "2 h".
pub fn fmt_span(ms: i64) -> String {
    let min = ms.max(0) / 60_000;
    match min {
        0 => "<1 min".into(),
        1..=59 => format!("{min} min"),
        _ => format!("{} h", min / 60),
    }
}

/// The line of one task: `shell: npm test · 4 min`, without the kind for a shell.
pub fn task_line(t: &BgTask, now: i64) -> String {
    let what = match t.kind.as_str() {
        "shell" => t.label.clone(),
        kind => format!("{kind}: {}", t.label),
    };
    format!("{what} · {}", fmt_span(now - t.started_at))
}

/// The line of one loop: `↻ every 10 min · next 14:20`, `↻ next 14:20`, `↻ goal: ship it`.
pub fn loop_line(l: &LoopInfo, now: i64) -> String {
    let mut parts = vec![];
    if !l.schedule.is_empty() {
        parts.push(l.schedule.clone());
    }
    if let Some(at) = l.next_run(now) {
        parts.push(format!("next {}", fmt_clock(at)));
    }
    if parts.is_empty() {
        parts.push("loop".into());
    }
    format!("↻ {}", parts.join(" · "))
}

/// The one line for a card or a picker row: the first task (`+2 more` for the others), else the
/// first loop. None when nothing is going.
pub fn summary(s: &Session, now: i64) -> Option<String> {
    let ts = tasks(s, now);
    if let Some(first) = ts.first() {
        let more = match ts.len() {
            1 => String::new(),
            n => format!(" +{} more", n - 1),
        };
        return Some(format!("background: {}{more}", task_line(first, now)));
    }
    loops(s, now).first().map(|l| loop_line(l, now))
}

/// The short marks for a picker row or a listing: `bg` (`bg 2` for several tasks), `↻` for a loop
/// and `RC` for a remote control link, separated by spaces; empty when there is none.
pub fn marker(s: &Session, now: i64) -> String {
    let mut marks = vec![];
    match tasks(s, now).len() {
        0 => {}
        1 => marks.push("bg".to_string()),
        n => marks.push(format!("bg {n}")),
    }
    if !loops(s, now).is_empty() {
        marks.push("↻".into());
    }
    if remote_url(s).is_some() {
        marks.push("RC".into());
    }
    marks.join(" ")
}

/// The link to the session on the web, when its Remote Control is on and it is open.
pub fn remote_url(s: &Session) -> Option<&str> {
    s.remote_url
        .as_deref()
        .filter(|u| s.live_status.is_some() && u.starts_with("https://"))
}

/// What a live `waiting` asks of the user, None when it is only a dialog the user opened or the
/// agent did not say.
pub fn live_ask(s: &Session) -> Option<&str> {
    s.live_wait
        .as_deref()
        .filter(|w| s.live_status.as_deref() == Some("waiting") && !w.eq_ignore_ascii_case(DIALOG))
}

/// The reason a live status gives for a local dialog the user opened: not an ask.
pub const DIALOG: &str = "dialog open";

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, kind: &str, label: &str, started_at: i64) -> BgTask {
        BgTask {
            id: id.into(),
            kind: kind.into(),
            label: label.into(),
            started_at,
        }
    }

    fn every(n: &str, started_at: i64) -> LoopInfo {
        LoopInfo {
            kind: "interval".into(),
            schedule: format!("every {n} min"),
            next_at: None,
            prompt: "check".into(),
            started_at,
            cron: Some(format!("*/{n} * * * *")),
        }
    }

    fn open(s: Signals) -> Session {
        Session {
            live_status: Some("idle".into()),
            signals: s,
            ..Session::default()
        }
    }

    #[test]
    fn a_label_is_one_short_line_without_control_characters() {
        assert_eq!(one_line("echo \x1b]0;x\x07 a\n\tb"), "echo ]0;x a b");
        assert_eq!(one_line(&"x".repeat(200)).chars().count(), LABEL_MAX);
    }

    #[test]
    fn a_closed_session_shows_nothing() {
        let mut s = open(Signals {
            tasks: vec![task("1", "shell", "npm test", 0)],
            loops: vec![every("10", 0)],
        });
        assert!(has_work(&s, 1_000));
        s.live_status = None;
        assert!(!has_work(&s, 1_000));
        assert_eq!(summary(&s, 1_000), None);
    }

    #[test]
    fn a_task_older_than_twelve_hours_is_dropped() {
        let s = open(Signals {
            tasks: vec![task("1", "shell", "npm test", 0)],
            loops: vec![],
        });
        assert!(has_work(&s, TASK_MAX_AGE_MS));
        assert!(!has_work(&s, TASK_MAX_AGE_MS + 1));
    }

    #[test]
    fn the_summary_names_the_first_task_and_counts_the_rest() {
        let now = 5 * 60_000;
        let mut s = open(Signals {
            tasks: vec![
                task("1", "shell", "npm test", 60_000),
                task("2", "agent", "review", 0),
                task("3", "monitor", "logs", 0),
            ],
            loops: vec![],
        });
        assert_eq!(
            summary(&s, now).as_deref(),
            Some("background: npm test · 4 min +2 more")
        );
        s.signals.tasks.truncate(2);
        s.signals.tasks.remove(0);
        assert_eq!(
            summary(&s, now).as_deref(),
            Some("background: agent: review · 5 min")
        );
    }

    #[test]
    fn a_loop_line_says_when_it_runs_next() {
        let now = 1_000_000_000_000;
        let l = every("10", now);
        let next = l.next_run(now).unwrap();
        assert!(next > now && next - now <= 10 * 60_000);
        assert_eq!(next % (10 * 60_000), 0);
        assert!(loop_line(&l, now).starts_with("↻ every 10 min · next "));
        let wake = LoopInfo {
            kind: "self_paced".into(),
            schedule: String::new(),
            next_at: Some(now + 60_000),
            prompt: "x".into(),
            started_at: now,
            cron: None,
        };
        assert!(loop_line(&wake, now).starts_with("↻ next "));
        let goal = LoopInfo {
            kind: "goal".into(),
            schedule: "goal: ship it".into(),
            next_at: None,
            prompt: String::new(),
            started_at: now,
            cron: None,
        };
        assert_eq!(loop_line(&goal, now), "↻ goal: ship it");
    }

    #[test]
    fn a_cron_that_is_not_every_n_minutes_has_no_computed_next_run() {
        let mut l = every("7", 0);
        assert_eq!(l.next_run(1_000), None);
        l.cron = Some("0 9 * * *".into());
        assert_eq!(l.next_run(1_000), None);
        l.cron = None;
        assert_eq!(l.next_run(1_000), None);
    }

    #[test]
    fn a_self_paced_loop_ends_when_it_does_not_wake_again() {
        let wake = LoopInfo {
            kind: "self_paced".into(),
            schedule: String::new(),
            next_at: Some(1_000),
            prompt: "x".into(),
            started_at: 0,
            cron: None,
        };
        let s = open(Signals {
            tasks: vec![],
            loops: vec![wake],
        });
        assert_eq!(loops(&s, 1_000 + WAKE_GRACE_MS).len(), 1);
        assert!(loops(&s, 1_001 + WAKE_GRACE_MS).is_empty());
    }

    #[test]
    fn an_interval_loop_expires_after_a_week() {
        let s = open(Signals {
            tasks: vec![],
            loops: vec![every("10", 0)],
        });
        assert_eq!(loops(&s, INTERVAL_MAX_AGE_MS).len(), 1);
        assert!(loops(&s, INTERVAL_MAX_AGE_MS + 1).is_empty());
    }

    #[test]
    fn the_marker_counts_tasks_and_shows_loops_and_remote_control() {
        let mut s = open(Signals::default());
        assert_eq!(marker(&s, 0), "");
        s.signals.tasks = vec![task("1", "shell", "a", 0)];
        assert_eq!(marker(&s, 0), "bg");
        s.signals.tasks.push(task("2", "agent", "b", 0));
        s.signals.loops = vec![every("10", 0)];
        s.remote_url = Some("https://claude.ai/code/x".into());
        assert_eq!(marker(&s, 0), "bg 2 ↻ RC");
    }

    #[test]
    fn a_remote_link_needs_an_open_session_and_https() {
        let mut s = open(Signals::default());
        s.remote_url = Some("https://claude.ai/code/session_01".into());
        assert!(remote_url(&s).is_some());
        s.remote_url = Some("javascript:alert(1)".into());
        assert_eq!(remote_url(&s), None);
        s.remote_url = Some("https://claude.ai/code/session_01".into());
        s.live_status = None;
        assert_eq!(remote_url(&s), None);
    }

    #[test]
    fn a_dialog_the_user_opened_is_not_an_ask() {
        let mut s = open(Signals::default());
        s.live_status = Some("waiting".into());
        s.live_wait = Some("permission prompt".into());
        assert_eq!(live_ask(&s), Some("permission prompt"));
        s.live_wait = Some("dialog open".into());
        assert_eq!(live_ask(&s), None);
        s.live_wait = Some("permission prompt".into());
        s.live_status = Some("idle".into());
        assert_eq!(live_ask(&s), None);
    }

    #[test]
    fn columns_round_trip_and_bad_text_is_empty() {
        let signals = Signals {
            tasks: vec![task("1", "shell", "x", 5)],
            loops: vec![every("10", 0)],
        };
        let text = to_column(&signals).unwrap();
        assert_eq!(from_column::<Signals>(Some(&text)), signals);
        let loops = signals.loops.clone();
        let text = to_column(&loops).unwrap();
        assert_eq!(from_column::<Vec<LoopInfo>>(Some(&text)), loops);
        assert_eq!(to_column(&Signals::default()), None);
        assert_eq!(to_column::<Vec<LoopInfo>>(&vec![]), None);
        assert!(from_column::<Signals>(Some("{{")).is_empty());
        assert!(from_column::<Signals>(None).is_empty());
    }
}
