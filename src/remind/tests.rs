use super::*;
use crate::ideas::{self, NewIdea};
use chrono::Duration;

const HOUR: i64 = 3_600_000;

fn add(db: &Db, id: &str, title: &str, last: i64) {
    db.conn
        .execute(
            "INSERT INTO session(id,title,last_activity_at,prompt_count) VALUES(?1,?2,?3,5)",
            params![id, title, last],
        )
        .unwrap();
}

fn session(db: &Db, id: &str) -> Session {
    db.session_by_id(id).unwrap().unwrap()
}

fn sid(id: &str) -> Target {
    Target::Session(id.into())
}

#[test]
fn a_reminder_is_upcoming_then_due_and_gone_once_you_went_back_to_it() {
    let db = Db::open_memory().unwrap();
    add(&db, "aaaaaaaa-1", "Waiting on Ana", 1_000);
    assert_eq!(of_session(&session(&db, "aaaaaaaa-1"), 5_000), None);
    set(&db, &sid("aaaaaaaa-1"), 10_000, Some("Ana's reply")).unwrap();
    let s = session(&db, "aaaaaaaa-1");
    let upcoming = of_session(&s, 5_000).unwrap();
    assert_eq!((upcoming.at, upcoming.due), (10_000, false));
    let due = of_session(&s, 10_000).unwrap();
    assert!(due.due);
    assert_eq!(due.reason(), "⏰ check on: Ana's reply");
    // activity after the time means you resumed it: the reminder is answered
    db.conn
        .execute("UPDATE session SET last_activity_at=10_001", [])
        .unwrap();
    assert_eq!(of_session(&session(&db, "aaaaaaaa-1"), 20_000), None);
    // activity before the time does not matter
    db.conn
        .execute("UPDATE session SET last_activity_at=9_000", [])
        .unwrap();
    assert!(of_session(&session(&db, "aaaaaaaa-1"), 20_000).unwrap().due);
}

#[test]
fn without_a_note_the_reason_says_you_asked() {
    assert_eq!(reason(None), "⏰ you asked to check on this");
}

#[test]
fn an_idea_stays_due_until_you_act_on_it() {
    let db = Db::open_memory().unwrap();
    let i = ideas::add(
        &db,
        &NewIdea {
            title: "Look at the queue",
            ..Default::default()
        },
    )
    .unwrap();
    let t = Target::resolve(&db, &format!("idea:{}", i.id)).unwrap();
    set(&db, &t, 2_000, None).unwrap();
    let i = ideas::get(&db, i.id).unwrap().unwrap();
    assert!(!of_idea(&i, 1_000).unwrap().due);
    assert!(of_idea(&i, 9_999_999).unwrap().due);
    clear(&db, &t).unwrap();
    assert_eq!(
        of_idea(&ideas::get(&db, i.id).unwrap().unwrap(), 5_000),
        None
    );
}

#[test]
fn a_new_time_snoozes_and_clear_removes_it() {
    let db = Db::open_memory().unwrap();
    add(&db, "aaaaaaaa-1", "t", 1);
    set(&db, &sid("aaaaaaaa-1"), 100, Some("first")).unwrap();
    db.conn
        .execute("UPDATE session SET remind_notified_at=100", [])
        .unwrap();
    set(&db, &sid("aaaaaaaa-1"), 900, None).unwrap();
    let s = session(&db, "aaaaaaaa-1");
    assert_eq!(
        (s.remind_at, s.remind_note, s.remind_notified_at),
        (Some(900), None, None)
    );
    clear(&db, &sid("aaaaaaaa-1")).unwrap();
    let s = session(&db, "aaaaaaaa-1");
    assert_eq!((s.remind_at, s.remind_note), (None, None));
    assert!(set(&db, &sid("nope"), 1, None).is_err());
}

#[test]
fn the_note_is_trimmed_and_kept_short() {
    let db = Db::open_memory().unwrap();
    add(&db, "aaaaaaaa-1", "t", 1);
    set(&db, &sid("aaaaaaaa-1"), 5, Some("   ")).unwrap();
    assert_eq!(session(&db, "aaaaaaaa-1").remind_note, None);
    set(
        &db,
        &sid("aaaaaaaa-1"),
        5,
        Some(&format!("  {}  ", "x".repeat(500))),
    )
    .unwrap();
    assert_eq!(
        session(&db, "aaaaaaaa-1")
            .remind_note
            .unwrap()
            .chars()
            .count(),
        NOTE_CHARS
    );
}

#[test]
fn moving_a_session_by_hand_answers_a_due_reminder_and_done_answers_any() {
    let db = Db::open_memory().unwrap();
    add(&db, "aaaaaaaa-1", "t", 1);
    set(&db, &sid("aaaaaaaa-1"), 5_000, None).unwrap();
    answered(&db, "aaaaaaaa-1", 1_000, false).unwrap();
    assert_eq!(
        session(&db, "aaaaaaaa-1").remind_at,
        Some(5_000),
        "not due yet"
    );
    answered(&db, "aaaaaaaa-1", 1_000, true).unwrap();
    assert_eq!(session(&db, "aaaaaaaa-1").remind_at, None, "done ends it");
    set(&db, &sid("aaaaaaaa-1"), 5_000, None).unwrap();
    answered(&db, "aaaaaaaa-1", 6_000, false).unwrap();
    assert_eq!(
        session(&db, "aaaaaaaa-1").remind_at,
        None,
        "a due one is answered"
    );
}

#[test]
fn targets_are_found_by_prefix_or_idea_id() {
    let db = Db::open_memory().unwrap();
    add(&db, "abcdef12-0000", "t", 1);
    let i = ideas::add(
        &db,
        &NewIdea {
            title: "i",
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        Target::resolve(&db, "abcdef").unwrap(),
        sid("abcdef12-0000")
    );
    assert_eq!(
        Target::resolve(&db, &format!("idea:{}", i.id)).unwrap(),
        Target::Idea(i.id)
    );
    for bad in ["idea:99", "idea:x", "zzzzzz", "abc"] {
        assert!(Target::resolve(&db, bad).is_err(), "{bad}");
    }
}

#[test]
fn the_list_has_due_ones_first_and_leaves_out_archived_and_answered_ones() {
    let db = Db::open_memory().unwrap();
    for (id, last) in [
        ("aaaaaaaa-1", 1),
        ("bbbbbbbb-2", 1),
        ("cccccccc-3", 1),
        ("dddddddd-4", 1),
    ] {
        add(&db, id, id, last);
    }
    set(&db, &sid("aaaaaaaa-1"), 9_000, None).unwrap();
    set(&db, &sid("bbbbbbbb-2"), 3_000, None).unwrap();
    set(&db, &sid("cccccccc-3"), 2_000, None).unwrap();
    set(&db, &sid("dddddddd-4"), 1_500, None).unwrap();
    db.conn
        .execute("UPDATE session SET archived=1 WHERE id='dddddddd-4'", [])
        .unwrap();
    db.conn
        .execute(
            "UPDATE session SET last_activity_at=2_500 WHERE id='cccccccc-3'",
            [],
        )
        .unwrap();
    let i = ideas::add(
        &db,
        &NewIdea {
            title: "an idea",
            ..Default::default()
        },
    )
    .unwrap();
    set(&db, &Target::Idea(i.id), 2_800, None).unwrap();
    let ids: Vec<String> = list(&db, 4_000)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(ids, ["idea:1", "bbbbbbbb-2", "aaaaaaaa-1"]);
    assert!(list(&Db::open_memory().unwrap(), 1).unwrap().is_empty());
}

#[test]
fn each_due_reminder_is_announced_once_and_again_after_a_new_time() {
    let db = Db::open_memory().unwrap();
    add(&db, "aaaaaaaa-1", "Waiting on Ana", 1);
    add(&db, "bbbbbbbb-2", "Later", 1);
    add(&db, "cccccccc-3", "Resumed", 1);
    set(&db, &sid("aaaaaaaa-1"), 1_000, Some("Ana's reply")).unwrap();
    set(&db, &sid("bbbbbbbb-2"), 90_000, None).unwrap();
    set(&db, &sid("cccccccc-3"), 1_000, None).unwrap();
    db.conn
        .execute(
            "UPDATE session SET last_activity_at=2_000 WHERE id='cccccccc-3'",
            [],
        )
        .unwrap();
    let mut seen: Vec<(String, String)> = vec![];
    let mut send = |t: &str, b: &str| seen.push((t.into(), b.into()));
    assert_eq!(notify_due(&db, 5_000, &mut send).unwrap(), 1);
    assert_eq!(notify_due(&db, 6_000, &mut send).unwrap(), 0, "once");
    set(&db, &sid("aaaaaaaa-1"), 7_000, None).unwrap();
    assert_eq!(
        notify_due(&db, 8_000, &mut send).unwrap(),
        1,
        "a snooze is a new reminder"
    );
    assert_eq!(
        seen,
        [
            (
                "Check on: Waiting on Ana".to_string(),
                "Ana's reply".to_string()
            ),
            (
                "Check on: Waiting on Ana".to_string(),
                "You asked to check on this".to_string()
            ),
        ]
    );
}

#[test]
fn a_row_that_cannot_be_resolved_does_not_stop_the_others() {
    let db = Db::open_memory().unwrap();
    // an id that reads as an idea id but is not a number
    add(&db, "idea:odd", "Odd one", 1);
    add(&db, "aaaaaaaa-1", "Fine", 1);
    for (id, at) in [("idea:odd", 1_000), ("aaaaaaaa-1", 1_500)] {
        db.conn
            .execute(
                "UPDATE session SET remind_at=?2 WHERE id=?1",
                params![id, at],
            )
            .unwrap();
    }
    let mut seen = vec![];
    let mut send = |t: &str, _: &str| seen.push(t.to_string());
    assert_eq!(notify_due(&db, 5_000, &mut send).unwrap(), 1);
    assert_eq!(seen, ["Check on: Fine"]);
}

#[test]
fn an_idea_is_announced_too() {
    let db = Db::open_memory().unwrap();
    let i = ideas::add(
        &db,
        &NewIdea {
            title: "Look at the queue",
            ..Default::default()
        },
    )
    .unwrap();
    set(&db, &Target::Idea(i.id), 1_000, Some("by Friday")).unwrap();
    let mut n = 0;
    let mut send = |t: &str, b: &str| {
        assert_eq!((t, b), ("Check on: Look at the queue", "by Friday"));
        n += 1;
    };
    assert_eq!(notify_due(&db, 2_000, &mut send).unwrap(), 1);
    assert_eq!(notify_due(&db, 3_000, &mut send).unwrap(), 0);
    assert_eq!(n, 1);
}

#[test]
fn an_old_database_gets_the_columns_and_keeps_its_rows() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("eye.db");
    {
        // what a build before reminders left: the tables without the new columns
        let db = Db::open(&path).unwrap();
        for table in ["session", "idea"] {
            for col in ["remind_at", "remind_note", "remind_notified_at"] {
                db.conn
                    .execute(&format!("ALTER TABLE {table} DROP COLUMN {col}"), [])
                    .unwrap();
            }
        }
        db.meta_set("schema_version", "10").unwrap();
        add(&db, "old-1", "old", 1);
        db.conn
            .execute("INSERT INTO idea(id,title) VALUES(1,'old idea')", [])
            .unwrap();
    }
    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("schema_version").unwrap(),
        Some(crate::db::SCHEMA_VERSION.to_string())
    );
    assert_eq!(session(&db, "old-1").remind_at, None);
    set(&db, &sid("old-1"), 5, None).unwrap();
    set(&db, &Target::Idea(1), 6, None).unwrap();
    drop(db);
    // opening it again changes nothing
    let db = Db::open(&path).unwrap();
    assert_eq!(session(&db, "old-1").remind_at, Some(5));
    assert_eq!(ideas::get(&db, 1).unwrap().unwrap().remind_at, Some(6));
}

#[test]
fn labels_say_today_tomorrow_the_weekday_or_the_date() {
    let at = |days: i64, h: u32| {
        let d = Local.with_ymd_and_hms(2026, 6, 10, h, 0, 0).unwrap() + Duration::days(days);
        d.timestamp_millis()
    };
    let now = at(0, 8);
    assert_eq!(label(at(0, 14), now), "today 14:00");
    assert_eq!(label(at(1, 9), now), "tomorrow 09:00");
    assert_eq!(label(at(3, 14), now), "Sat 14:00");
    assert_eq!(label(at(10, 9), now), "20 Jun 09:00");
    assert_eq!(label(at(300, 9), now), "6 Apr 2027 09:00");
    assert_eq!(label(now + HOUR, now), "today 09:00");
}
