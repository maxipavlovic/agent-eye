//! "Check on this": a date on a session or an idea that brings it back to Needs you. It is for the
//! reply that does not come and the idea to look at by a day. The date is local to the machine; no
//! calendar or service is involved.
//!
//! A reminder is due once its time has come and the session has not had activity since, because
//! activity means you went back to it. `when.rs` reads the words for a time.

pub mod notices;
mod when;

pub use when::{parse, parse_local};

use crate::db::Db;
use crate::ideas;
use crate::model::{Idea, Session};
use crate::pick::session_title;
use crate::util::head_chars;
use anyhow::{ensure, Context, Result};
use chrono::{Datelike, Local, TimeZone};
use rusqlite::{params, types::Value};

/// The longest note kept.
const NOTE_CHARS: usize = 200;

/// A reminder this many days ahead or nearer is shown on the card and the row; a further one only
/// in the panel and the lists.
pub const SOON_DAYS: i64 = 3;

/// A reminder that still matters: set for later, or due now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remind {
    pub at: i64,
    pub note: Option<String>,
    pub due: bool,
}

impl Remind {
    /// The line a due card carries.
    pub fn reason(&self) -> String {
        reason(self.note.as_deref())
    }

    /// One line for a preview or `eye show`: why it is due, or when it will be and the note.
    pub fn summary(&self, now: i64) -> String {
        match (self.due, &self.note) {
            (true, _) => self.reason(),
            (false, Some(n)) => format!("⏰ {} · {n}", label(self.at, now)),
            (false, None) => format!("⏰ {}", label(self.at, now)),
        }
    }
}

/// The reason line of a due reminder.
pub fn reason(note: Option<&str>) -> String {
    match note {
        Some(n) => format!("⏰ check on: {n}"),
        None => "⏰ you asked to check on this".into(),
    }
}

/// The reminder of a session, or None when there is none or it was answered: the session had
/// activity after the time.
pub fn of_session(s: &Session, now: i64) -> Option<Remind> {
    of(
        s.remind_at,
        s.remind_note.as_deref(),
        s.last_activity_at.unwrap_or(0),
        now,
    )
}

/// The reminder of an idea. An idea has no activity, so it stays until you act on it.
pub fn of_idea(i: &Idea, now: i64) -> Option<Remind> {
    of(i.remind_at, i.remind_note.as_deref(), 0, now)
}

/// Is a reminder set for `at` due: its time has come and there was no activity since.
pub fn is_due(at: Option<i64>, last_activity: i64, now: i64) -> bool {
    at.is_some_and(|at| at <= now && last_activity <= at)
}

fn of(at: Option<i64>, note: Option<&str>, last_activity: i64, now: i64) -> Option<Remind> {
    let at = at?;
    let due = is_due(Some(at), last_activity, now);
    (at > now || due).then(|| Remind {
        at,
        note: note.map(str::to_string),
        due,
    })
}

/// What a reminder is set on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Session(String),
    Idea(i64),
}

impl Target {
    /// A full session id, a unique prefix of one, or `idea:<n>`.
    pub fn resolve(db: &Db, text: &str) -> Result<Target> {
        match text.trim().strip_prefix("idea:") {
            Some(n) => {
                let n: i64 = n
                    .parse()
                    .with_context(|| format!("'{text}' is not an idea id"))?;
                ensure!(ideas::get(db, n)?.is_some(), "no idea {n}");
                Ok(Target::Idea(n))
            }
            None => Ok(Target::Session(db.resolve_id(text)?)),
        }
    }

    /// The id as people type it: the session id, or `idea:<n>`.
    pub fn id(&self) -> String {
        match self {
            Target::Session(id) => id.clone(),
            Target::Idea(n) => format!("idea:{n}"),
        }
    }

    /// What it is called.
    pub fn title(&self, db: &Db) -> Result<String> {
        match self {
            Target::Session(id) => Ok(session_title(
                &db.session_by_id(id)?.context("no such session")?,
            )),
            Target::Idea(n) => Ok(ideas::get(db, *n)?.context("no such idea")?.title),
        }
    }

    fn table(&self) -> &'static str {
        match self {
            Target::Session(_) => "session",
            Target::Idea(_) => "idea",
        }
    }

    fn key(&self) -> Value {
        match self {
            Target::Session(id) => Value::Text(id.clone()),
            Target::Idea(n) => Value::Integer(*n),
        }
    }
}

/// Set the reminder, replacing an earlier one (that is how it is snoozed). The note is trimmed;
/// an empty one is none.
pub fn set(db: &Db, target: &Target, at: i64, note: Option<&str>) -> Result<()> {
    let note = note
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| head_chars(n, NOTE_CHARS));
    let changed = db.conn.execute(
        &format!(
            "UPDATE {} SET remind_at=?2, remind_note=?3, remind_notified_at=NULL WHERE id=?1",
            target.table()
        ),
        params![target.key(), at, note],
    )?;
    ensure!(changed > 0, "nothing to set a reminder on");
    Ok(())
}

/// Remove the reminder.
pub fn clear(db: &Db, target: &Target) -> Result<()> {
    db.conn.execute(
        &format!(
            "UPDATE {} SET remind_at=NULL, remind_note=NULL, remind_notified_at=NULL WHERE id=?1",
            target.table()
        ),
        [target.key()],
    )?;
    Ok(())
}

/// You acted on a session by putting it in a column by hand: a due reminder is answered. With
/// `any` (Done, Archive) one that is not due yet goes too.
pub fn answered(db: &Db, id: &str, now: i64, any: bool) -> Result<()> {
    db.conn.execute(
        "UPDATE session SET remind_at=NULL, remind_note=NULL, remind_notified_at=NULL
         WHERE id=?1 AND remind_at IS NOT NULL AND (?3 OR remind_at<=?2)",
        params![id, now, any],
    )?;
    Ok(())
}

/// A reminder with what a list needs to show it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The session id, or `idea:<n>`.
    pub id: String,
    pub title: String,
    pub remind: Remind,
    pub notified_at: Option<i64>,
}

/// Every reminder that matters (not archived, not answered), due ones first, each group by time.
pub fn list(db: &Db, now: i64) -> Result<Vec<Entry>> {
    let mut v: Vec<Entry> = vec![];
    for s in db.all_sessions()?.iter().filter(|s| !s.archived) {
        if let Some(remind) = of_session(s, now) {
            v.push(Entry {
                id: s.id.clone(),
                title: session_title(s),
                remind,
                notified_at: s.remind_notified_at,
            });
        }
    }
    for i in ideas::list_open(db)? {
        if let Some(remind) = of_idea(&i, now) {
            v.push(Entry {
                id: format!("idea:{}", i.id),
                title: i.title.clone(),
                remind,
                notified_at: i.remind_notified_at,
            });
        }
    }
    v.sort_by_key(|e| (!e.remind.due, e.remind.at));
    Ok(v)
}

/// When a reminder is, in words: `today 14:00`, `tomorrow 09:00`, `Fri 14:00` within the week,
/// else the date.
pub fn label(at: i64, now: i64) -> String {
    let (Some(a), Some(n)) = (
        Local.timestamp_millis_opt(at).single(),
        Local.timestamp_millis_opt(now).single(),
    ) else {
        return "?".into();
    };
    let time = a.format("%H:%M");
    match (a.date_naive() - n.date_naive()).num_days() {
        0 => format!("today {time}"),
        1 => format!("tomorrow {time}"),
        2..=6 => format!("{} {time}", a.format("%a")),
        _ if a.year() == n.year() => format!("{} {time}", a.format("%-d %b")),
        _ => format!("{} {time}", a.format("%-d %b %Y")),
    }
}

/// Tell about each due reminder once: `send` shows a notice (title, text). A reminder counts as
/// told even when no notice could be shown, so a machine without a desktop is not tried again on
/// every tick. A row that cannot be resolved is reported on standard error and skipped, so it does
/// not hold back the others. Returns how many were told.
pub fn notify_due(db: &Db, now: i64, send: &mut dyn FnMut(&str, &str)) -> Result<usize> {
    let mut told = 0;
    for e in list(db, now)? {
        if !e.remind.due || e.notified_at.is_some_and(|n| n >= e.remind.at) {
            continue;
        }
        let target = match Target::resolve(db, &e.id) {
            Ok(t) => t,
            Err(err) => {
                eprintln!("eye: reminder of {} skipped: {err:#}", e.id);
                continue;
            }
        };
        db.conn.execute(
            &format!(
                "UPDATE {} SET remind_notified_at=?2 WHERE id=?1",
                target.table()
            ),
            params![target.key(), now],
        )?;
        let text = e
            .remind
            .note
            .clone()
            .unwrap_or_else(|| "You asked to check on this".into());
        send(&format!("Check on: {}", head_chars(&e.title, 80)), &text);
        told += 1;
    }
    Ok(told)
}

#[cfg(test)]
mod tests;
