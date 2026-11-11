use super::*;
use crate::agents::claude::Claude;
use crate::agents::codex::Codex;
use std::sync::Arc;

/// Claude Code is the only agent the daemon tests need.
fn agents() -> Agents {
    Agents::from_vec(vec![Arc::new(Claude::new("/nonexistent/c".into()))])
}

fn run_hook(p: &Paths, event: &str, payload: &[u8]) {
    hook::run(p, agents().first().unwrap().as_ref(), event, payload);
}

fn env() -> (tempfile::TempDir, Paths, Db) {
    let t = tempfile::tempdir().unwrap();
    let p = Paths {
        home: t.path().join("h"),
    };
    p.ensure_home().unwrap();
    let db = Db::open(&p.db_path()).unwrap();
    (t, p, db)
}

fn add_session(db: &Db, id: &str, last: i64) {
    db.conn
        .execute(
            "INSERT INTO session(id,kind,transcript_state,last_activity_at,prompt_count) VALUES(?1,'interactive','present',?2,3)",
            params![id, last],
        )
        .unwrap();
}

#[test]
fn a_due_reminder_is_announced_once_and_not_when_notices_are_off() {
    let (_t, p, db) = env();
    add_session(&db, "s1", 100);
    db.conn
        .execute("UPDATE session SET remind_at=200, title='Ana'", [])
        .unwrap();
    let seen = std::cell::RefCell::new(vec![]);
    let mut send = |t: &str, _: &str| seen.borrow_mut().push(t.to_string());
    let mut off = Config::default();
    off.remind.notify = false;
    notify_reminders(&db, &p, &off, &mut send);
    assert!(seen.borrow().is_empty());
    let mut on = Config::default();
    on.remind.notify = true;
    notify_reminders(&db, &p, &on, &mut send);
    notify_reminders(&db, &p, &on, &mut send);
    assert_eq!(*seen.borrow(), ["Check on: Ana"]);
}

#[test]
fn a_switch_made_while_the_daemon_runs_counts_at_once() {
    let (_t, p, db) = env();
    add_session(&db, "s1", 100);
    db.conn
        .execute("UPDATE session SET remind_at=200, title='Ana'", [])
        .unwrap();
    let seen = std::cell::RefCell::new(vec![]);
    let mut send = |t: &str, _: &str| seen.borrow_mut().push(t.to_string());
    let started_with = Config::default();
    notify_reminders(&db, &p, &started_with, &mut send);
    assert!(seen.borrow().is_empty(), "off unless asked");
    crate::remind::notices::set(&p, true).unwrap();
    notify_reminders(&db, &p, &started_with, &mut send);
    assert_eq!(*seen.borrow(), ["Check on: Ana"]);
}

#[test]
fn events_set_waiting_and_ended() {
    let (_t, p, db) = env();
    add_session(&db, "s1", 100);
    run_hook(
        &p,
        "Stop",
        br#"{"session_id":"s1","last_assistant_message":"Want me to continue?"}"#.as_slice(),
    );
    assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
    let s = db.session_by_id("s1").unwrap().unwrap();
    assert!(s.waiting_since.is_some());
    assert_eq!(
        s.waiting_note.as_deref(),
        Some("answer: Want me to continue?")
    );
    run_hook(
        &p,
        "UserPromptSubmit",
        br#"{"session_id":"s1","prompt":"yes"}"#.as_slice(),
    );
    run_hook(
        &p,
        "Stop",
        br#"{"session_id":"s1","last_assistant_message":"All done."}"#.as_slice(),
    );
    run_hook(
        &p,
        "SessionEnd",
        br#"{"session_id":"s1","reason":"other"}"#.as_slice(),
    );
    assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 3);
    assert_eq!(
        drain_queue(&db, &p, &agents()).unwrap(),
        0,
        "offset is kept"
    );
    let s = db.session_by_id("s1").unwrap().unwrap();
    assert!(s.waiting_since.is_none() && s.ended_at.is_some());
    assert_eq!(s.waiting_note, None);
    let n: i64 = db
        .conn
        .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 4);
    assert!(db.meta_get("last_hook_ts").unwrap().is_some());
}

#[test]
fn permission_request_and_garbage_lines() {
    let (_t, p, db) = env();
    add_session(&db, "s2", 100);
    run_hook(
        &p,
        "PermissionRequest",
        br#"{"session_id":"s2","tool_name":"Bash"}"#.as_slice(),
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(hook::queue_path(&p))
        .unwrap()
        .write_all(b"garbage\n")
        .unwrap();
    assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
    assert!(db
        .session_by_id("s2")
        .unwrap()
        .unwrap()
        .waiting_since
        .is_some());
}

#[test]
fn the_waiting_note_says_what_is_asked() {
    let (_t, p, db) = env();
    add_session(&db, "s3", 100);
    db.conn
        .execute("UPDATE session SET live_status='waiting' WHERE id='s3'", [])
        .unwrap();
    let note = |p: &Paths, db: &Db| {
        drain_queue(db, p, &agents()).unwrap();
        let s = db.session_by_id("s3").unwrap().unwrap();
        (s.waiting_for().map(str::to_string), s.next_line())
    };
    run_hook(&p, "PermissionRequest", br#"{"session_id":"s3","tool_name":"AskUserQuestion","tool_input":{"questions":[{"question":"Which repo first?"}]}}"#.as_slice());
    // the notification that follows keeps the precise note
    run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"permission_prompt","message":"Claude needs your permission"}"#.as_slice());
    let (w, next) = note(&p, &db);
    assert_eq!(w.as_deref(), Some("answer: Which repo first?"));
    assert_eq!(next.as_deref(), Some("You: answer: Which repo first?"));
    assert!(db.session_by_id("s3").unwrap().unwrap().blocked());
    run_hook(&p, "UserPromptSubmit", br#"{"session_id":"s3"}"#.as_slice());
    run_hook(&p, "Stop", br#"{"session_id":"s3","last_assistant_message":"Tests pass. Merged it. Shall I also tag the release?"}"#.as_slice());
    assert_eq!(
        note(&p, &db).0.as_deref(),
        Some("answer: Shall I also tag the release?")
    );
    // a finished reply that asks something waits on you but does not block Claude
    assert!(!db.session_by_id("s3").unwrap().unwrap().blocked());
    run_hook(
        &p,
        "PermissionRequest",
        br#"{"session_id":"s3","tool_name":"Bash","tool_input":{"command":"git push"}}"#.as_slice(),
    );
    assert_eq!(note(&p, &db).0.as_deref(), Some("allow Bash: git push"));
    // working again (or closed): no note, the written next step shows
    db.conn
        .execute(
            "UPDATE session SET live_status='busy', next_step='Sam: review' WHERE id='s3'",
            [],
        )
        .unwrap();
    assert_eq!(note(&p, &db), (None, Some("Sam: review".into())));
    db.conn
        .execute("UPDATE session SET live_status=NULL WHERE id='s3'", [])
        .unwrap();
    assert_eq!(note(&p, &db).0, None);
    run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#.as_slice());
    db.conn
        .execute("UPDATE session SET live_status='waiting' WHERE id='s3'", [])
        .unwrap();
    // "waiting for your input" after a finished reply changes nothing
    assert_eq!(note(&p, &db).0.as_deref(), Some("allow Bash: git push"));
    run_hook(&p, "UserPromptSubmit", br#"{"session_id":"s3"}"#.as_slice());
    run_hook(
        &p,
        "Stop",
        br#"{"session_id":"s3","last_assistant_message":"Done."}"#.as_slice(),
    );
    run_hook(&p, "Notification", br#"{"session_id":"s3","notification_type":"idle_prompt","message":"Claude is waiting for your input"}"#.as_slice());
    assert_eq!(note(&p, &db), (None, Some("Sam: review".into())));
    let s3 = db.session_by_id("s3").unwrap().unwrap();
    assert!(s3.waiting_since.is_none() && !s3.blocked());
}

#[test]
fn stored_waits_are_recomputed_under_the_current_rules() {
    let (_t, _p, db) = env();
    let ev = |sid: &str, ts: i64, kind: &str, payload: &str| {
        db.conn
            .execute(
                "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,?3,?4)",
                params![sid, ts, kind, payload],
            )
            .unwrap();
    };
    // a prompt mid-task, recorded before notes existed
    add_session(&db, "s4", 100);
    ev("s4", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
    ev("s4", 2, "UserPromptSubmit", "{}");
    ev(
        "s4",
        3,
        "PermissionRequest",
        r#"{"tool_name":"AskUserQuestion"}"#,
    );
    ev(
        "s4",
        4,
        "Notification",
        r#"{"notification_type":"permission_prompt"}"#,
    );
    // a finished reply, marked waiting by the idle notification under the old rule
    add_session(&db, "s5", 100);
    ev("s5", 5, "Stop", r#"{"last_message_tail":"All done."}"#);
    ev(
        "s5",
        6,
        "Notification",
        r#"{"notification_type":"idle_prompt"}"#,
    );
    db.conn
        .execute(
            "UPDATE session SET waiting_since=4, live_status='waiting' WHERE id='s4'",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE session SET waiting_since=6, waiting_note='reply to Claude', live_status='waiting' WHERE id='s5'",
            [],
        )
        .unwrap();
    assert_eq!(refresh_waits(&db, &agents()).unwrap(), 2);
    let s4 = db.session_by_id("s4").unwrap().unwrap();
    assert_eq!(
        (s4.waiting_for(), s4.blocked()),
        (Some("answer Claude's question in the session"), true)
    );
    let s5 = db.session_by_id("s5").unwrap().unwrap();
    assert_eq!(
        (s5.waiting_since, s5.waiting_for(), s5.blocked()),
        (None, None, false)
    );
    assert_eq!(
        refresh_waits(&db, &agents()).unwrap(),
        1,
        "only s4 still waits"
    );
}

fn codex_agents() -> Agents {
    Agents::from_vec(vec![Arc::new(Codex::new("/nonexistent/x".into()))])
}

fn codex_hook(p: &Paths, event: &str, payload: &[u8]) {
    hook::run(p, codex_agents().first().unwrap().as_ref(), event, payload);
}

#[test]
fn a_tool_that_ran_ends_a_prompt_wait_and_not_a_question() {
    let (_t, p, db) = env();
    add_session(&db, "c1", 100);
    db.conn
        .execute("UPDATE session SET agent='codex' WHERE id='c1'", [])
        .unwrap();
    let wait = |db: &Db| {
        drain_queue(db, &p, &codex_agents()).unwrap();
        let s = db.session_by_id("c1").unwrap().unwrap();
        (s.waiting_note, s.waiting_prompt)
    };
    codex_hook(
        &p,
        "PermissionRequest",
        br#"{"session_id":"c1","tool_name":"Bash","tool_input":{"command":"git push"}}"#.as_slice(),
    );
    assert_eq!(wait(&db), (Some("allow Bash: git push".into()), true));
    codex_hook(
        &p,
        "PostToolUse",
        br#"{"session_id":"c1","tool_name":"Bash"}"#.as_slice(),
    );
    assert_eq!(
        wait(&db),
        (None, false),
        "the tool ran: the prompt was answered"
    );
    // a reply that asks is a wait of its own, and a tool that ran does not answer it
    codex_hook(
        &p,
        "Stop",
        br#"{"session_id":"c1","last_assistant_message":"Done. Shall I tag it?"}"#.as_slice(),
    );
    codex_hook(
        &p,
        "PostToolUse",
        br#"{"session_id":"c1","tool_name":"Bash"}"#.as_slice(),
    );
    assert_eq!(wait(&db), (Some("answer: Shall I tag it?".into()), false));
}

#[test]
fn a_replay_applies_a_tool_that_ran_the_same_way() {
    let (_t, _p, db) = env();
    let ev = |sid: &str, ts: i64, kind: &str, payload: &str| {
        db.conn
            .execute(
                "INSERT INTO event(session_id, ts, kind, payload) VALUES(?1,?2,?3,?4)",
                params![sid, ts, kind, payload],
            )
            .unwrap();
    };
    // answered: nothing is waiting any more
    add_session(&db, "c2", 100);
    ev("c2", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
    ev("c2", 2, "PostToolUse", "{}");
    // a prompt, answered, then a reply that asks, then more tools: the question stays
    add_session(&db, "c3", 100);
    ev("c3", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
    ev("c3", 2, "PostToolUse", "{}");
    ev(
        "c3",
        3,
        "Stop",
        r#"{"last_message_tail":"Shall I tag it?"}"#,
    );
    ev("c3", 4, "PostToolUse", "{}");
    // still on the prompt
    add_session(&db, "c4", 100);
    ev("c4", 1, "PermissionRequest", r#"{"tool_name":"Bash"}"#);
    for id in ["c2", "c3", "c4"] {
        db.conn
            .execute(
                "UPDATE session SET agent='codex', waiting_since=1, waiting_prompt=1, live_status='idle' WHERE id=?1",
                [id],
            )
            .unwrap();
    }
    assert_eq!(refresh_waits(&db, &codex_agents()).unwrap(), 3);
    let got = |id: &str| {
        let s = db.session_by_id(id).unwrap().unwrap();
        (
            s.waiting_since,
            s.waiting_for().map(str::to_string),
            s.blocked(),
        )
    };
    assert_eq!(got("c2"), (None, None, false));
    assert_eq!(
        got("c3"),
        (Some(3), Some("answer: Shall I tag it?".into()), false)
    );
    assert_eq!(got("c4"), (Some(1), Some("allow Bash".into()), true));
}

#[test]
fn quiet_by_time_or_session_end() {
    let (_t, p, db) = env();
    let now = 10_000_000;
    add_session(&db, "idle-old", now - 11 * 60_000);
    add_session(&db, "fresh", now - 60_000);
    add_session(&db, "ended", now - 120_000);
    db.conn
        .execute(
            "INSERT INTO event(session_id,ts,kind,payload) VALUES('ended',?1,'SessionEnd','{}')",
            [now - 100_000],
        )
        .unwrap();
    db.conn
        .execute("UPDATE session SET live_status='busy' WHERE id='fresh'", [])
        .unwrap();
    let q: Vec<String> = quiet_sessions(&db, &Config::default(), now)
        .unwrap()
        .into_iter()
        .map(|x| x.0)
        .collect();
    assert_eq!(q, vec!["ended", "idle-old"]);
    let _ = p;
}

#[test]
fn lock_is_exclusive() {
    let (_t, p, _db) = env();
    let l = Lock::acquire(&p).unwrap();
    assert!(Lock::acquire(&p).is_err());
    assert_eq!(running_pid(&p), Some(std::process::id() as i64));
    drop(l);
    // a program another test starts at this moment holds a copy of the descriptor until it
    // execs, and the lock with it: wait that out
    let again = (0..200).find_map(|_| {
        Lock::acquire(&p).ok().or_else(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            None
        })
    });
    assert!(again.is_some());
}

#[test]
fn events_of_a_disabled_agent_are_logged_and_change_nothing() {
    let (_t, p, db) = env();
    add_session(&db, "s6", 100);
    let line =
        r#"{"ts":5,"event":"Stop","session_id":"s6","agent":"other","last_message_tail":"Go on?"}"#;
    std::fs::create_dir_all(hook::queue_path(&p).parent().unwrap()).unwrap();
    std::fs::write(hook::queue_path(&p), format!("{line}\n")).unwrap();
    assert_eq!(drain_queue(&db, &p, &agents()).unwrap(), 1);
    assert!(db
        .session_by_id("s6")
        .unwrap()
        .unwrap()
        .waiting_since
        .is_none());
    let n: i64 = db
        .conn
        .query_row("SELECT count(*) FROM event", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}
