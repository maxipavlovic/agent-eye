//! The night shift: limits, planning, the scheduler with a fake machine, isolation in a real
//! temporary repo, and the command lines.

use super::clock::{test_ms, Window};
use super::launch::{self, Started};
use super::sched::{self, Collected, Sys};
use super::*;
use crate::config::Config;
use crate::loose::System;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

fn cfg_with(extra: &str) -> Config {
    Config::parse(&format!("[night]\nenabled = true\n{extra}\n")).unwrap()
}

fn two_agents() -> Config {
    Config::parse("[agents]\nenabled = [\"claude\", \"codex\"]\n[night]\nenabled = true\n").unwrap()
}

fn at(day: &str, hm: &str) -> i64 {
    test_ms(day, hm, 2)
}

/// A machine that only exists in memory. The zone is UTC+2.
struct Fake {
    now: Cell<i64>,
    busy: Cell<bool>,
    alive: RefCell<HashSet<i64>>,
    next_pid: Cell<i64>,
    started: RefCell<Vec<i64>>,
    terminated: RefCell<Vec<i64>>,
    exits: RefCell<HashMap<i64, i32>>,
    tails: RefCell<HashMap<i64, String>>,
    sessions: RefCell<HashMap<i64, String>>,
    other: Option<String>,
    notes: RefCell<Vec<String>>,
    no_start_time: Cell<bool>,
}

impl Fake {
    fn new(day: &str, hm: &str) -> Fake {
        Fake {
            now: Cell::new(at(day, hm)),
            busy: Cell::new(false),
            alive: Default::default(),
            next_pid: Cell::new(1000),
            started: Default::default(),
            terminated: Default::default(),
            exits: Default::default(),
            tails: Default::default(),
            sessions: Default::default(),
            other: None,
            notes: Default::default(),
            no_start_time: Cell::new(false),
        }
    }
    fn at(&self, day: &str, hm: &str) {
        self.now.set(at(day, hm));
    }
    /// The process of a run ends with this exit code.
    fn end(&self, run: &Run, code: i32) {
        self.alive.borrow_mut().remove(&run.pid.unwrap());
        self.exits.borrow_mut().insert(run.id, code);
    }
}

impl Sys for Fake {
    fn now(&self) -> i64 {
        self.now.get()
    }
    fn offset(&self, _: i64) -> i64 {
        2 * 3_600_000
    }
    fn interactive_busy(&self, _: i64) -> bool {
        self.busy.get()
    }
    fn alive(&self, pid: i64, _: Option<i64>) -> bool {
        self.alive.borrow().contains(&pid)
    }
    fn terminate(&self, pid: i64) {
        self.terminated.borrow_mut().push(pid);
    }
    fn start(&self, run: &Run) -> anyhow::Result<Started> {
        let pid = self.next_pid.get();
        self.next_pid.set(pid + 1);
        self.alive.borrow_mut().insert(pid);
        self.started.borrow_mut().push(run.id);
        Ok(Started {
            pid,
            pid_start: (!self.no_start_time.get()).then_some(7),
            iso: iso::Isolation {
                cwd: "/n/wt".into(),
                root: format!("/n/{}/wt", run.id).into(),
                branch: Some(format!("night/x-{}", run.id)),
                base_ref: Some("abc".into()),
            },
            session_id: self.sessions.borrow().get(&run.id).cloned(),
        })
    }
    fn exit_code(&self, run: &Run) -> Option<i32> {
        self.exits.borrow().get(&run.id).copied()
    }
    fn log_tail(&self, run: &Run) -> String {
        self.tails
            .borrow()
            .get(&run.id)
            .cloned()
            .unwrap_or_default()
    }
    fn collect(&self, _: &mut Db, run: &Run) -> Collected {
        Collected {
            session_id: self.sessions.borrow().get(&run.id).cloned(),
            report: Some("all done".into()),
            diff_stat: " a.txt | 1 +".into(),
        }
    }
    fn other_agent(&self, _: &str) -> Option<String> {
        self.other.clone()
    }
    fn notify(&self, title: &str, body: &str) {
        self.notes.borrow_mut().push(format!("{title}: {body}"));
    }
}

/// A db with `n` planned runs of idea runs for the night of 2026-11-21.
fn db_with(n: usize, night: &str) -> Db {
    let db = Db::open_memory().unwrap();
    for i in 0..n {
        db.conn
            .execute(
                "INSERT INTO night_run(kind, idea_id, agent, folder, prompt, night, created_at)
                 VALUES('idea', ?1, 'claude', '/x', 'do it', ?2, 1)",
                params![i as i64 + 1, night],
            )
            .unwrap();
    }
    db
}

fn status_of(db: &Db, id: i64) -> String {
    get(db, id).unwrap().unwrap().status
}

fn tick(db: &mut Db, f: &Fake, cfg: &Config) -> sched::Pass {
    sched::tick(db, f, cfg).unwrap()
}

// ---------- limits ----------

#[test]
fn limits_parse_in_every_form_and_refuse_nonsense() {
    assert_eq!(Limit::parse("30%").unwrap(), Limit::WeeklyPercent(30));
    assert_eq!(Limit::parse("2M").unwrap(), Limit::Tokens(2_000_000));
    assert_eq!(Limit::parse("500k").unwrap(), Limit::Tokens(500_000));
    assert_eq!(Limit::parse("90m").unwrap(), Limit::Minutes(90));
    assert_eq!(Limit::parse("2h").unwrap(), Limit::Minutes(120));
    assert_eq!(Limit::parse("1.5M").unwrap(), Limit::Tokens(1_500_000));
    for bad in ["", "x", "0%", "101%", "-3m", "90", "m"] {
        assert!(Limit::parse(bad).is_err(), "{bad}");
    }
    let l = Limit::Minutes(90);
    assert_eq!(Limit::from_parts(Some(l.kind()), Some(l.value())), Some(l));
}

// ---------- planning ----------

fn idea(db: &Db, folder: Option<&str>, prompt: Option<&str>) -> i64 {
    crate::ideas::add(
        db,
        &crate::ideas::NewIdea {
            title: "t",
            prompt,
            folder,
            ..Default::default()
        },
    )
    .unwrap()
    .id
}

#[test]
fn an_idea_needs_a_folder_and_a_prompt_and_says_which() {
    let db = Db::open_memory().unwrap();
    let cfg = Config::default();
    let agents = crate::agents::enabled(&cfg);
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap();
    let no_folder = idea(&db, None, Some("p"));
    let e = plan_idea(&db, &cfg, &agents, no_folder, &PlanOpts::default()).unwrap_err();
    assert!(format!("{e}").contains("no folder"), "{e}");
    let no_prompt = idea(&db, Some(dir), None);
    let e = plan_idea(&db, &cfg, &agents, no_prompt, &PlanOpts::default()).unwrap_err();
    assert!(format!("{e}").contains("no prompt"), "{e}");
    let ok = idea(&db, Some(dir), Some("write a readme"));
    let r = plan_idea(&db, &cfg, &agents, ok, &PlanOpts::default()).unwrap();
    assert_eq!(
        (r.status.as_str(), r.kind.as_str(), r.agent.as_str()),
        ("planned", "idea", "claude")
    );
    assert_eq!(planned_count(&db).unwrap(), 1);
    assert_eq!(get(&db, r.id).unwrap().unwrap().prompt, "write a readme");
}

#[test]
fn a_limit_the_data_cannot_check_is_refused_with_the_reason() {
    let db = Db::open_memory().unwrap();
    let cfg = Config::default();
    let agents = crate::agents::enabled(&cfg);
    let tmp = tempfile::tempdir().unwrap();
    let id = idea(&db, Some(tmp.path().to_str().unwrap()), Some("p"));
    let weekly = PlanOpts {
        limit: Some(Limit::WeeklyPercent(80)),
        ..Default::default()
    };
    let e = plan_idea(&db, &cfg, &agents, id, &weekly).unwrap_err();
    assert!(
        format!("{e}").contains("statusline") || format!("{e}").contains("eye install"),
        "{e}"
    );
    assert!(format!("{e}").contains("no weekly plan data"), "{e}");
    // tokens and minutes can always be checked
    for l in [Limit::Tokens(1000), Limit::Minutes(5)] {
        plan_idea(
            &db,
            &cfg,
            &agents,
            id,
            &PlanOpts {
                limit: Some(l),
                ..Default::default()
            },
        )
        .unwrap();
    }
    // with data it is accepted
    crate::usage::limits::record(
        &db.conn,
        "claude",
        &crate::usage::PlanLimits {
            seen_at: 1,
            plan: None,
            windows: vec![weekly_window(10.0, None)],
        },
    )
    .unwrap();
    plan_idea(&db, &cfg, &agents, id, &weekly).unwrap();
}

fn weekly_window(pct: f64, resets: Option<i64>) -> crate::usage::LimitWindow {
    crate::usage::LimitWindow {
        name: "seven_day".into(),
        used_percent: pct,
        window_minutes: Some(10_080),
        resets_at: resets,
    }
}

#[test]
fn switching_needs_a_second_agent_and_never_applies_to_a_continued_session() {
    let db = Db::open_memory().unwrap();
    let cfg = Config::default();
    let agents = crate::agents::enabled(&cfg);
    let tmp = tempfile::tempdir().unwrap();
    let id = idea(&db, Some(tmp.path().to_str().unwrap()), Some("p"));
    let e = plan_idea(
        &db,
        &cfg,
        &agents,
        id,
        &PlanOpts {
            switch: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(format!("{e}").contains("second enabled agent"), "{e}");
    let cfg2 = two_agents();
    let both = crate::agents::enabled(&cfg2);
    plan_idea(
        &db,
        &cfg2,
        &both,
        id,
        &PlanOpts {
            switch: true,
            ..Default::default()
        },
    )
    .unwrap();
}

fn session_row(db: &Db, id: &str, cwd: &str, transcript: Option<&str>) {
    db.conn
        .execute(
            "INSERT INTO session(id, agent, cwd, transcript_path, transcript_state, started_at, last_activity_at)
             VALUES(?1,'claude',?2,?3,'present',1,1)",
            params![id, cwd, transcript],
        )
        .unwrap();
}

#[test]
fn continue_tonight_needs_a_prompt_and_refuses_a_switch() {
    let db = Db::open_memory().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::default();
    let agents = crate::agents::enabled(&cfg);
    let sid = "11111111-2222-4333-8444-555555555555";
    session_row(&db, sid, tmp.path().to_str().unwrap(), None);
    let e = plan_session(&db, &cfg, &agents, "111111", &PlanOpts::default()).unwrap_err();
    assert!(format!("{e}").contains("needs a prompt"), "{e}");
    let e = plan_session(
        &db,
        &cfg,
        &agents,
        "111111",
        &PlanOpts {
            prompt: Some("go".into()),
            switch: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(format!("{e}").contains("ideas"), "{e}");
    let r = plan_session(
        &db,
        &cfg,
        &agents,
        "111111",
        &PlanOpts {
            prompt: Some("finish the tests".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.kind, "session");
    assert_eq!(r.session_id.as_deref(), Some(sid));
}

#[test]
fn cancel_only_works_on_a_planned_run() {
    let db = db_with(2, "2026-11-21");
    cancel(&db, 1).unwrap();
    assert_eq!(status_of(&db, 1), "skipped");
    assert!(cancel(&db, 1).is_err());
    db.conn
        .execute("UPDATE night_run SET status='running' WHERE id=2", [])
        .unwrap();
    assert!(cancel(&db, 2).is_err());
    assert!(set_limit(&db, &crate::agents::enabled(&Config::default()), 2, None).is_err());
}

// ---------- the scheduler ----------

#[test]
fn nothing_starts_until_the_night_shift_is_switched_on() {
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    let off = Config::parse("[night]\nenabled = false\n").unwrap();
    assert!(tick(&mut db, &f, &off).started.is_empty());
    assert_eq!(status_of(&db, 1), "planned");
}

#[test]
fn runs_start_only_inside_the_window_edges() {
    let cfg = cfg_with("");
    for (hm, starts) in [
        ("00:59", false),
        ("01:00", true),
        ("06:59", true),
        ("07:00", false),
        ("12:00", false),
    ] {
        let mut db = db_with(1, "2026-11-21");
        let f = Fake::new("2026-11-21", hm);
        assert_eq!(!tick(&mut db, &f, &cfg).started.is_empty(), starts, "{hm}");
    }
}

#[test]
fn a_window_that_crosses_midnight_works_on_both_sides() {
    let cfg = cfg_with("window = \"23:00-05:00\"");
    for (day, hm, starts) in [
        ("2026-11-21", "22:59", false),
        ("2026-11-21", "23:00", true),
        ("2026-11-22", "02:00", true),
        ("2026-11-22", "04:59", true),
        ("2026-11-22", "05:00", false),
    ] {
        let mut db = db_with(1, "2026-11-21");
        let f = Fake::new(day, hm);
        assert_eq!(
            !tick(&mut db, &f, &cfg).started.is_empty(),
            starts,
            "{day} {hm}"
        );
    }
    // a run planned for the coming night waits for it
    let mut db = db_with(1, "2026-11-22");
    let f = Fake::new("2026-11-21", "23:30");
    assert!(tick(&mut db, &f, &cfg).started.is_empty());
}

#[test]
fn a_run_for_a_later_night_waits_and_one_missed_runs_in_the_next_window() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-22");
    let f = Fake::new("2026-11-21", "03:00");
    assert!(tick(&mut db, &f, &cfg).started.is_empty());
    f.at("2026-11-22", "03:00");
    assert_eq!(tick(&mut db, &f, &cfg).started, vec![1]);
}

#[test]
fn the_quiet_rule_holds_new_runs_back_but_not_running_ones() {
    let cfg = cfg_with("");
    let mut db = db_with(2, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    f.busy.set(true);
    assert!(tick(&mut db, &f, &cfg).started.is_empty());
    f.busy.set(false);
    assert_eq!(tick(&mut db, &f, &cfg).started.len(), 2);
    // a session you start now does not stop the runs that already go
    f.busy.set(true);
    let p = tick(&mut db, &f, &cfg);
    assert!(p.finished.is_empty() && p.stopped.is_empty());
    assert_eq!(status_of(&db, 1), "running");
}

#[test]
fn planned_runs_never_block_each_other_up_to_parallel() {
    let cfg = cfg_with("parallel = 2");
    let mut db = db_with(3, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    assert_eq!(tick(&mut db, &f, &cfg).started, vec![1, 2]);
    assert_eq!(status_of(&db, 3), "planned");
    // nothing more while both go
    assert!(tick(&mut db, &f, &cfg).started.is_empty());
    // one ends: the third starts on the pass that finds the room
    let r1 = get(&db, 1).unwrap().unwrap();
    f.end(&r1, 0);
    let p = tick(&mut db, &f, &cfg);
    assert_eq!(p.finished, vec![1]);
    assert_eq!(p.started, vec![3]);
    assert_eq!(status_of(&db, 1), "done");
    assert!(f
        .notes
        .borrow()
        .iter()
        .any(|n| n == "Night shift: 1 run finished"));
}

fn token_session(db: &Db, id: &str, tokens: i64) {
    session_row(db, id, "/n/1/wt", None);
    db.conn
        .execute(
            "UPDATE session SET tok_in=?2 WHERE id=?1",
            params![id, tokens],
        )
        .unwrap();
}

fn running_run(db: &Db, f: &Fake, cfg: &Config, limit: Option<&str>, sid: &str) -> Run {
    f.sessions.borrow_mut().insert(1, sid.into());
    if let Some(l) = limit {
        let l = Limit::parse(l).unwrap();
        db.conn
            .execute(
                "UPDATE night_run SET limit_kind=?1, limit_value=?2 WHERE id=1",
                params![l.kind(), l.value()],
            )
            .unwrap();
    }
    let _ = cfg;
    get(db, 1).unwrap().unwrap()
}

#[test]
fn a_token_budget_stops_the_run_and_says_why() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    running_run(&db, &f, &cfg, Some("2M"), "s-1");
    token_session(&db, "s-1", 1_000_000);
    tick(&mut db, &f, &cfg);
    assert_eq!(status_of(&db, 1), "running", "under the budget");
    db.conn
        .execute("UPDATE session SET tok_in=2500000 WHERE id='s-1'", [])
        .unwrap();
    let p = tick(&mut db, &f, &cfg);
    assert_eq!(p.stopped, vec![1]);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "stopped");
    assert!(r.why.unwrap().contains("token budget"), "why");
    assert_eq!(f.terminated.borrow().len(), 1);
    // while the process lingers it is not finished; when it is gone it is, and stays stopped
    assert!(r.ended_at.is_none());
    f.end(&get(&db, 1).unwrap().unwrap(), 143);
    tick(&mut db, &f, &cfg);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "stopped");
    assert!(r.ended_at.is_some());
}

#[test]
fn a_time_budget_stops_the_run() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    running_run(&db, &f, &cfg, Some("30m"), "s-1");
    tick(&mut db, &f, &cfg);
    f.now.set(f.now.get() + 29 * 60_000);
    assert!(tick(&mut db, &f, &cfg).stopped.is_empty());
    f.now.set(f.now.get() + 2 * 60_000);
    assert_eq!(tick(&mut db, &f, &cfg).stopped, vec![1]);
    assert!(get(&db, 1)
        .unwrap()
        .unwrap()
        .why
        .unwrap()
        .contains("time limit 30 min"));
}

#[test]
fn a_weekly_percent_limit_stops_the_run_from_the_stored_plan_data() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    running_run(&db, &f, &cfg, Some("80%"), "s-1");
    let record = |db: &Db, pct, resets| {
        crate::usage::limits::record(
            &db.conn,
            "claude",
            &crate::usage::PlanLimits {
                seen_at: f.now.get(),
                plan: None,
                windows: vec![weekly_window(pct, resets)],
            },
        )
        .unwrap();
    };
    record(&db, 50.0, None);
    tick(&mut db, &f, &cfg);
    assert_eq!(status_of(&db, 1), "running");
    // stale data from a window that has reset says nothing
    record(&db, 95.0, Some(f.now.get() - 1000));
    assert!(tick(&mut db, &f, &cfg).stopped.is_empty());
    f.now.set(f.now.get() + 1000);
    record(&db, 81.0, Some(f.now.get() + 3_600_000));
    assert_eq!(tick(&mut db, &f, &cfg).stopped, vec![1]);
    assert_eq!(
        get(&db, 1).unwrap().unwrap().why.as_deref(),
        Some("weekly limit 80% reached (81% used)")
    );
}

#[test]
fn a_run_that_ran_out_of_plan_starts_again_on_the_other_agent_when_allowed() {
    let cfg = two_agents();
    let mut db = db_with(2, "2026-11-21");
    db.conn
        .execute("UPDATE night_run SET switch_ok=1 WHERE id=1", [])
        .unwrap();
    let mut f = Fake::new("2026-11-21", "03:00");
    f.other = Some("codex".into());
    tick(&mut db, &f, &cfg);
    let (r1, r2) = (get(&db, 1).unwrap().unwrap(), get(&db, 2).unwrap().unwrap());
    f.tails
        .borrow_mut()
        .insert(1, "Error: rate limit reached, try again at 9pm".into());
    f.tails
        .borrow_mut()
        .insert(2, "Error: rate limit reached".into());
    f.end(&r1, 1);
    f.end(&r2, 1);
    tick(&mut db, &f, &cfg);
    let r1 = get(&db, 1).unwrap().unwrap();
    assert_eq!(r1.status, "failed");
    assert!(r1.why.unwrap().contains("started again on codex"));
    // the one with switch allowed has a successor on codex, and it started in the same pass
    let new = get(&db, 3).unwrap().unwrap();
    assert_eq!(
        (new.agent.as_str(), new.prompt.as_str()),
        ("codex", "do it")
    );
    assert_eq!(new.status, "running");
    // the one without it just failed, with the reason
    let r2 = get(&db, 2).unwrap().unwrap();
    assert_eq!(r2.status, "failed");
    assert!(r2.why.unwrap().contains("rate limit"));
    assert!(get(&db, 4).unwrap().is_none());
}

#[test]
fn a_run_whose_plan_is_already_out_starts_on_the_other_agent() {
    let cfg = two_agents();
    let mut db = db_with(1, "2026-11-21");
    db.conn
        .execute("UPDATE night_run SET switch_ok=1 WHERE id=1", [])
        .unwrap();
    let mut f = Fake::new("2026-11-21", "03:00");
    f.other = Some("codex".into());
    crate::usage::limits::record(
        &db.conn,
        "claude",
        &crate::usage::PlanLimits {
            seen_at: f.now.get(),
            plan: None,
            windows: vec![weekly_window(100.0, Some(f.now.get() + 1_000_000))],
        },
    )
    .unwrap();
    tick(&mut db, &f, &cfg);
    assert_eq!(get(&db, 1).unwrap().unwrap().agent, "codex");
}

#[test]
fn a_daemon_restart_finds_running_runs_again_and_finishes_the_ended_ones() {
    let cfg = cfg_with("");
    let mut db = db_with(2, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    tick(&mut db, &f, &cfg);
    let (a, b) = (get(&db, 1).unwrap().unwrap(), get(&db, 2).unwrap().unwrap());
    // a new scheduler (a new Fake) that knows only which pids are alive: run 1 is, run 2 is gone
    let g = Fake::new("2026-11-21", "03:10");
    g.alive.borrow_mut().insert(a.pid.unwrap());
    g.exits.borrow_mut().insert(2, 0);
    let p = tick(&mut db, &g, &cfg);
    assert_eq!(p.finished, vec![b.id]);
    assert!(
        p.started.is_empty(),
        "the running one is not started a second time"
    );
    assert_eq!(status_of(&db, 1), "running");
    assert_eq!(status_of(&db, 2), "done");
    assert_eq!(g.started.borrow().len(), 0);
}

#[test]
fn a_finished_run_puts_its_session_in_needs_you_with_the_next_step() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    f.sessions.borrow_mut().insert(1, "s-1".into());
    session_row(&db, "s-1", "/n/1/wt", None);
    tick(&mut db, &f, &cfg);
    f.end(&get(&db, 1).unwrap().unwrap(), 0);
    tick(&mut db, &f, &cfg);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(
        (r.status.as_str(), r.report.as_deref()),
        ("done", Some("all done"))
    );
    assert_eq!(r.diff_stat.as_deref(), Some(" a.txt | 1 +"));
    assert_eq!(r.run_session_id.as_deref(), Some("s-1"));
    assert!(r.ended_at.is_some());
    let s = db.session_by_id("s-1").unwrap().unwrap();
    assert_eq!(s.column_manual.as_deref(), Some("needs_you"));
    assert_eq!(s.next_step.as_deref(), Some(NEXT_STEP));
    assert_eq!(of_session(&db, "s-1").unwrap().unwrap().id, 1);
}

#[test]
fn a_failed_run_keeps_the_last_line_of_its_log_as_the_reason() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    tick(&mut db, &f, &cfg);
    f.tails
        .borrow_mut()
        .insert(1, "starting\nboom: no such file\n\n".into());
    f.end(&get(&db, 1).unwrap().unwrap(), 2);
    tick(&mut db, &f, &cfg);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "failed");
    assert_eq!(r.why.as_deref(), Some("exit 2: boom: no such file"));
}

#[test]
fn rate_limit_words_are_found_and_ordinary_logs_are_not() {
    assert!(sched::rate_limited("API Error: 429 too many requests"));
    assert!(sched::rate_limited("You've hit your usage limit"));
    assert!(!sched::rate_limited("tests passed, nothing to see"));
}

#[test]
fn the_window_default_is_one_to_seven() {
    assert_eq!(Window::default().text(), "01:00-07:00");
}

// ---------- isolation, in a real temporary repo ----------

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn repo(dir: &Path) {
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    std::fs::write(dir.join("sub/b.txt"), "b\n").unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "first"]);
}

fn run_in(root: &Path, id: i64, folder: &Path, isolation: &iso::Isolation) -> Run {
    let _ = root;
    Run {
        id,
        kind: "idea".into(),
        folder: folder.to_string_lossy().into(),
        worktree: Some(isolation.root.to_string_lossy().into()),
        branch: isolation.branch.clone(),
        base_ref: isolation.base_ref.clone(),
        status: "done".into(),
        ..Run::default()
    }
}

#[test]
fn a_repo_run_gets_a_worktree_on_a_new_branch_from_head() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("proj");
    repo(&orig);
    let head = git(&orig, &["rev-parse", "HEAD"]);
    let night = tmp.path().join("eye/night");
    let p = System::new();
    let i = iso::prepare(&p, &night.join("1"), &orig, "2026-11-21", "add-readme", 1).unwrap();
    assert_eq!(i.branch.as_deref(), Some("night/20261121-add-readme"));
    assert_eq!(i.base_ref.as_deref(), Some(head.as_str()));
    assert!(i.root.join("a.txt").is_file());
    assert_ne!(i.root, orig);
    assert_eq!(
        git(&i.root, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "night/20261121-add-readme"
    );
    // the original folder is untouched and stays on its branch
    assert_eq!(git(&orig, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    // a second run with the same slug the same night gets its own branch
    let j = iso::prepare(&p, &night.join("2"), &orig, "2026-11-21", "add-readme", 2).unwrap();
    assert_eq!(j.branch.as_deref(), Some("night/20261121-add-readme-2"));
    // a run started in a sub-folder starts in the same sub-folder of its worktree
    let k = iso::prepare(
        &p,
        &night.join("3"),
        &orig.join("sub"),
        "2026-11-21",
        "sub",
        3,
    )
    .unwrap();
    assert_eq!(k.cwd, k.root.join("sub"));
}

#[test]
fn the_diff_stat_lists_commits_edits_and_new_files() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("proj");
    repo(&orig);
    let p = System::new();
    let i = iso::prepare(
        &p,
        &tmp.path().join("eye/night/1"),
        &orig,
        "2026-11-21",
        "x",
        1,
    )
    .unwrap();
    std::fs::write(i.root.join("a.txt"), "one\ntwo\n").unwrap();
    git(&i.root, &["commit", "-qam", "edit"]);
    std::fs::write(i.root.join("b2.txt"), "new\n").unwrap();
    let run = run_in(tmp.path(), 1, &orig, &i);
    let d = iso::diff_stat(&p, &run);
    assert!(d.contains("a.txt"), "{d}");
    assert!(d.contains("1 new file(s) not added to git"), "{d}");
    assert!(d.contains("b2.txt"), "{d}");
    assert!(iso::has_uncommitted(&p, &run));
}

#[test]
fn discard_removes_worktree_and_branch_and_protects_uncommitted_work() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("proj");
    repo(&orig);
    let night = tmp.path().join("eye/night");
    let p = System::new();
    let i = iso::prepare(&p, &night.join("1"), &orig, "2026-11-21", "x", 1).unwrap();
    let run = run_in(tmp.path(), 1, &orig, &i);
    std::fs::write(i.root.join("scratch.txt"), "unsaved\n").unwrap();
    let e = iso::discard(&p, &night, &run, false).unwrap_err();
    assert!(format!("{e}").contains("uncommitted"), "{e}");
    assert!(i.root.exists());
    let said = iso::discard(&p, &night, &run, true).unwrap();
    assert!(said.contains("night/20261121-x"), "{said}");
    assert!(!i.root.exists());
    assert!(git(&orig, &["branch", "--list", "night/*"]).is_empty());
    assert!(!git(&orig, &["worktree", "list"]).contains("night"));
    // the original repo still has its files
    assert!(orig.join("a.txt").is_file());
}

#[test]
fn discard_never_removes_a_folder_outside_the_night_folder_and_not_a_live_run() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("proj");
    repo(&orig);
    let night = tmp.path().join("eye/night");
    std::fs::create_dir_all(&night).unwrap();
    let p = System::new();
    let mut run = Run {
        id: 1,
        folder: orig.to_string_lossy().into(),
        worktree: Some(orig.to_string_lossy().into()),
        status: "done".into(),
        ..Run::default()
    };
    let e = iso::discard(&p, &night, &run, true).unwrap_err();
    assert!(format!("{e}").contains("not inside"), "{e}");
    assert!(orig.join("a.txt").is_file());
    run.status = "running".into();
    assert!(iso::discard(&p, &night, &run, true).is_err());
}

#[test]
fn a_plain_folder_is_copied_without_the_skip_list_and_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("plain");
    for d in ["node_modules/x", ".git", "target", ".venv", "dist", "src"] {
        std::fs::create_dir_all(orig.join(d)).unwrap();
        std::fs::write(orig.join(d).join("f.txt"), "f").unwrap();
    }
    std::fs::write(orig.join("keep.txt"), "k").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", orig.join("link")).unwrap();
    let p = System::new();
    let dir = tmp.path().join("eye/night/1");
    let i = iso::prepare(&p, &dir, &orig, "2026-11-21", "x", 1).unwrap();
    assert_eq!(i.root, dir.join("work"));
    assert!(i.branch.is_none() && i.base_ref.is_none());
    assert!(i.root.join("keep.txt").is_file() && i.root.join("src/f.txt").is_file());
    for d in ["node_modules", ".git", "target", ".venv", "dist"] {
        assert!(!i.root.join(d).exists(), "{d}");
    }
    assert!(!i.root.join("link").exists());
    // the diff of a copy lists what changed
    std::fs::write(i.root.join("keep.txt"), "changed").unwrap();
    std::fs::write(i.root.join("new.txt"), "n").unwrap();
    let run = run_in(tmp.path(), 1, &orig, &i);
    let d = iso::diff_stat(&p, &run);
    assert!(
        d.contains("changed  keep.txt") && d.contains("new      new.txt"),
        "{d}"
    );
    assert!(!d.contains("src/f.txt"));
    // discard removes the copy
    let night = tmp.path().join("eye/night");
    iso::discard(&p, &night, &run, false).unwrap();
    assert!(!i.root.exists());
    assert!(orig.join("keep.txt").is_file());
}

#[test]
fn a_plain_folder_over_100_mb_is_refused_when_planned_and_what_is_skipped_does_not_count() {
    let tmp = tempfile::tempdir().unwrap();
    let big = tmp.path().join("big");
    std::fs::create_dir_all(big.join("node_modules")).unwrap();
    // sparse files: they cost no disk but have a length
    std::fs::File::create(big.join("node_modules/huge.bin"))
        .unwrap()
        .set_len(500 << 20)
        .unwrap();
    std::fs::File::create(big.join("ok.bin"))
        .unwrap()
        .set_len(50 << 20)
        .unwrap();
    iso::check_plain_size(&big).unwrap();
    std::fs::File::create(big.join("more.bin"))
        .unwrap()
        .set_len(60 << 20)
        .unwrap();
    let e = iso::check_plain_size(&big).unwrap_err();
    assert!(format!("{e}").contains("bigger than 100 MB"), "{e}");
    // planning refuses it, with the reason
    let db = Db::open_memory().unwrap();
    let cfg = Config::default();
    let agents = crate::agents::enabled(&cfg);
    let id = idea(&db, Some(big.to_str().unwrap()), Some("p"));
    assert!(plan_idea(&db, &cfg, &agents, id, &PlanOpts::default()).is_err());
    // a big repo is fine: it gets a worktree, not a copy
    repo(&big);
    iso::check_plain_size(&big).unwrap();
}

// ---------- command lines ----------

fn run_of(agent: &str, kind: &str) -> Run {
    Run {
        id: 4,
        kind: kind.into(),
        agent: agent.into(),
        prompt: "make it better".into(),
        session_id: (kind == "session").then(|| "sess-1".to_string()),
        ..Run::default()
    }
}

fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let i = args
        .iter()
        .position(|a| a == flag)
        .unwrap_or_else(|| panic!("{flag} in {args:?}"));
    &args[i + 1]
}

#[test]
fn the_claude_command_denies_every_way_out_and_names_its_session() {
    let cfg = Config::default();
    let s = launch::build(
        &cfg,
        &run_of("claude", "idea"),
        "claude",
        Path::new("/w"),
        Some("uuid-1"),
        None,
    );
    assert_eq!(
        s.args[..2],
        ["-p".to_string(), "make it better".to_string()]
    );
    assert_eq!(after(&s.args, "--session-id"), "uuid-1");
    assert_eq!(s.session_id.as_deref(), Some("uuid-1"));
    assert_eq!(after(&s.args, "--permission-mode"), "auto");
    assert!(!s.args.contains(&"--allowedTools".to_string()));
    let deny = after(&s.args, "--disallowedTools");
    for w in [
        "git push",
        "git remote",
        "Bash(gh*)",
        "Bash(ssh*)",
        "npm publish",
    ] {
        assert!(deny.contains(w), "{w} in {deny}");
    }
    assert!(!s
        .args
        .iter()
        .any(|a| a.contains("force") || a == "--dangerously-skip-permissions"));
    assert!(!s.args.contains(&"--resume".to_string()));
}

#[test]
fn accept_edits_mode_adds_an_allow_list_and_a_model_when_set() {
    let cfg =
        Config::parse("[night]\nclaude_permission = \"acceptEdits\"\nclaude_model = \"haiku\"\n")
            .unwrap();
    let s = launch::build(
        &cfg,
        &run_of("claude", "idea"),
        "claude",
        Path::new("/w"),
        Some("u"),
        None,
    );
    assert_eq!(after(&s.args, "--permission-mode"), "acceptEdits");
    assert!(after(&s.args, "--allowedTools").contains("Bash(cargo test*)"));
    assert!(after(&s.args, "--disallowedTools").contains("git push"));
    assert_eq!(after(&s.args, "--model"), "haiku");
}

#[test]
fn a_prompt_that_starts_with_a_dash_is_not_read_as_a_flag() {
    let cfg = Config::default();
    let mut r = run_of("claude", "idea");
    r.prompt = "--dangerously-skip-permissions".into();
    let s = launch::build(&cfg, &r, "claude", Path::new("/w"), Some("u"), None);
    assert_eq!(s.args[1], "Task: --dangerously-skip-permissions");
}

#[test]
fn continue_tonight_forks_the_session_for_claude_and_codex() {
    let cfg = Config::default();
    let s = launch::build(
        &cfg,
        &run_of("claude", "session"),
        "claude",
        Path::new("/w"),
        Some("new-1"),
        None,
    );
    let i = s.args.iter().position(|a| a == "--resume").unwrap();
    assert_eq!(s.args[i + 1], "sess-1");
    assert_eq!(s.args[i + 2], "--fork-session");
    assert_eq!(after(&s.args, "--session-id"), "new-1");
    let c = launch::build(
        &cfg,
        &run_of("codex", "session"),
        "codex",
        Path::new("/w"),
        None,
        None,
    );
    assert_eq!(c.args[..2], ["exec".to_string(), "fork".to_string()]);
    assert!(c.args.contains(&"sess-1".to_string()));
    assert!(c.args.iter().any(|a| a.contains("workspace-write")));
    let told = c.args.last().unwrap();
    assert!(
        told.ends_with("make it better") && told.contains("change nothing in"),
        "{told}"
    );
}

#[test]
fn the_codex_command_is_sandboxed_to_the_worktree_without_network() {
    let cfg = Config::parse("[night]\ncodex_model = \"gpt-mini\"\n").unwrap();
    let c = launch::build(
        &cfg,
        &run_of("codex", "idea"),
        "codex",
        Path::new("/w/x"),
        None,
        None,
    );
    assert_eq!(c.args[0], "exec");
    assert_eq!(after(&c.args, "--sandbox"), "workspace-write");
    assert_eq!(after(&c.args, "--cd"), "/w/x");
    assert_eq!(after(&c.args, "-m"), "gpt-mini");
    assert!(!c
        .args
        .iter()
        .any(|a| a.contains("danger") || a.contains("bypass")));
    assert!(c.session_id.is_none());
}

#[test]
fn the_claude_project_folder_name_replaces_every_odd_character() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path().join("a b.c");
    std::fs::create_dir_all(&d).unwrap();
    let real = d.canonicalize().unwrap();
    let name = launch::claude_project_dir(&d);
    assert!(name.starts_with('-') && name.ends_with("a-b-c"), "{name}");
    assert_eq!(name.len(), real.to_string_lossy().len());
}

#[test]
fn continuing_puts_the_transcript_where_claude_looks_from_the_worktree() {
    let tmp = tempfile::tempdir().unwrap();
    let claude = tmp.path().join("claude");
    let orig = tmp.path().join("proj");
    std::fs::create_dir_all(&orig).unwrap();
    let sid = "11111111-2222-4333-8444-555555555555";
    let pdir = claude
        .join("projects")
        .join(launch::claude_project_dir(&orig));
    std::fs::create_dir_all(&pdir).unwrap();
    let t = pdir.join(format!("{sid}.jsonl"));
    std::fs::write(&t, "{\"type\":\"user\"}\n").unwrap();
    let db = Db::open_memory().unwrap();
    session_row(&db, sid, orig.to_str().unwrap(), t.to_str());
    let cfg = Config::with_claude_dir(&claude);
    let agents = crate::agents::enabled(&cfg);
    let paths = Paths {
        home: tmp.path().join("eye"),
    };
    let wt = tmp.path().join("eye/night/1/wt");
    std::fs::create_dir_all(&wt).unwrap();
    let run = Run {
        kind: "session".into(),
        session_id: Some(sid.into()),
        agent: "claude".into(),
        ..Run::default()
    };
    launch::stage_transcript(&db, &paths, &agents, &run, &wt).unwrap();
    let dest = claude
        .join("projects")
        .join(launch::claude_project_dir(&wt))
        .join(format!("{sid}.jsonl"));
    assert_eq!(
        std::fs::read_to_string(dest).unwrap(),
        "{\"type\":\"user\"}\n"
    );
    // the original is where it was
    assert!(t.is_file());
}

// ---------- a process that really runs ----------

#[test]
fn a_spawned_run_leads_its_own_group_logs_and_leaves_an_exit_code() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("fake");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"args: $*\"\necho oops >&2\nexit 3\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let spec = launch::Spec {
        program: script.to_string_lossy().into(),
        args: vec!["-p".into(), "hello world".into()],
        cwd: tmp.path().into(),
        session_id: None,
    };
    let (log, exit) = (tmp.path().join("log"), tmp.path().join("exit"));
    let pid = launch::spawn(&spec, &log, &exit).unwrap();
    for _ in 0..100 {
        if exit.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(std::fs::read_to_string(&exit).unwrap().trim(), "3");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.contains("args: -p hello world") && text.contains("oops"),
        "{text}"
    );
    assert!(pid > 1);
}

// ---------- the barrier between a Claude run and the user's folders ----------

use super::barrier;

/// Run the hook command of the barrier on one tool call, as Claude Code does: JSON on stdin.
fn hook_says(cmd: &str, input: &str) -> i32 {
    use std::io::Write;
    let mut c = Command::new("sh")
        .args(["-c", cmd])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    c.wait().unwrap().code().unwrap()
}

fn call(key: &str, value: &str) -> String {
    serde_json::json!({ "tool_input": { key: value, "other": "x" }, "cwd": "/w" }).to_string()
}

#[test]
fn the_hook_blocks_the_file_tools_and_bash_in_a_guarded_folder_only() {
    let guard = vec![
        PathBuf::from("/Users/me/my code"),
        PathBuf::from("/Users/me/app"),
    ];
    let files = barrier::hook_command(&guard, false).unwrap();
    let bash = barrier::hook_command(&guard, true).unwrap();
    for p in [
        "/Users/me/app/src/a.rs",
        "/Users/me/my code/new.txt",
        "/run/wt/../../Users/me/app/x",
        "../app/x",
    ] {
        assert_eq!(hook_says(&files, &call("file_path", p)), 2, "{p}");
    }
    assert_eq!(
        hook_says(&files, &call("notebook_path", "/Users/me/app/n.ipynb")),
        2
    );
    for p in [
        "/run/wt/a.rs",
        "/Users/me/app2/a.rs",
        "/Users/me/application",
    ] {
        assert_eq!(hook_says(&files, &call("file_path", p)), 0, "{p}");
    }
    for c in [
        "touch /Users/me/app/x",
        "cd /Users/me/app && ls",
        "echo z > '/Users/me/my code/x'",
        "ls /Users/me/app; ls",
        "(cd /Users/me/app)",
    ] {
        assert_eq!(hook_says(&bash, &call("command", c)), 2, "{c}");
    }
    for c in [
        "ls",
        "git add -A && git commit -m x",
        "touch /Users/me/app2/x",
        "cat /run/wt/app/x",
    ] {
        assert_eq!(hook_says(&bash, &call("command", c)), 0, "{c}");
    }
    // a name the hook cannot quote safely is left to the rules, and with nothing left there is no hook
    assert!(barrier::hook_command(&[PathBuf::from("/a'b")], true).is_none());
}

#[test]
fn a_worktree_run_may_write_its_own_folder_and_its_git_but_not_the_original() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("orig").canonicalize().unwrap_or_else(|_| {
        std::fs::create_dir_all(tmp.path().join("orig")).unwrap();
        tmp.path().join("orig").canonicalize().unwrap()
    });
    repo(&orig);
    let wt = tmp.path().canonicalize().unwrap().join("run/wt");
    std::fs::create_dir_all(wt.parent().unwrap()).unwrap();
    git(
        &orig,
        &["worktree", "add", &wt.to_string_lossy(), "-b", "night/x"],
    );
    let b = barrier::plan(&wt, std::slice::from_ref(&orig));
    let has = |v: &[PathBuf], p: PathBuf| v.contains(&p);
    assert!(has(&b.allow_write, wt.clone()));
    assert!(has(&b.allow_write, orig.join(".git/objects")));
    assert!(has(&b.allow_write, orig.join(".git/refs/heads/night")));
    assert!(b
        .allow_write
        .iter()
        .any(|p| p.starts_with(orig.join(".git/worktrees"))));
    // every entry but .git, and the parts of .git that change behaviour
    assert!(has(&b.deny_write, orig.join("a.txt")));
    assert!(has(&b.deny_write, orig.join("sub")));
    assert!(has(&b.deny_write, orig.join(".git/hooks")));
    assert!(has(&b.deny_write, orig.join(".git/config")));
    assert!(!has(&b.deny_write, orig.join(".git")));
    assert!(!has(&b.deny_write, orig.clone()));
    assert!(b
        .deny_edit
        .contains(&format!("Edit(/{}/**)", orig.join("sub").to_string_lossy())));
    assert!(b
        .deny_edit
        .contains(&format!("Edit(/{})", orig.join("a.txt").to_string_lossy())));
    assert_eq!(b.guard, vec![orig.clone()]);
    // a run that started in a sub-folder protects the whole repo
    let sub_only = barrier::plan(&wt, &[orig.join("sub")]);
    assert!(sub_only.guard.contains(&orig));
    // the commit a run makes works with exactly what it may write? git itself decides; here only
    // that the pointer file was read: the worktree's own git folder is allowed
    let gitdir = std::fs::read_to_string(wt.join(".git")).unwrap();
    let gd = gitdir
        .trim()
        .trim_start_matches("gitdir:")
        .trim()
        .to_string();
    assert!(b.allow_write.iter().any(|p| p.to_string_lossy() == gd));
}

#[test]
fn a_plain_folder_is_denied_whole() {
    let tmp = tempfile::tempdir().unwrap();
    let orig = tmp.path().join("plain");
    std::fs::create_dir_all(&orig).unwrap();
    std::fs::write(orig.join("a.txt"), "x").unwrap();
    let work = tmp.path().join("run/work");
    std::fs::create_dir_all(&work).unwrap();
    let b = barrier::plan(&work, std::slice::from_ref(&orig));
    assert!(b.deny_write.contains(&orig));
    assert!(b
        .deny_edit
        .contains(&format!("Edit(/{}/**)", orig.to_string_lossy())));
    assert!(b.allow_write.contains(&work));
}

#[test]
fn the_settings_of_a_run_turn_the_sandbox_on_and_keep_the_push_deny_list() {
    let b = barrier::Barrier {
        allow_write: vec![PathBuf::from("/run/wt")],
        deny_write: vec![PathBuf::from("/orig/a.txt")],
        deny_edit: vec!["Edit(//orig/a.txt)".into()],
        guard: vec![PathBuf::from("/orig")],
    };
    let v: serde_json::Value = serde_json::from_str(&barrier::settings(
        &b,
        &launch::DENY,
        &Config::default().night.network_list(),
        true,
    ))
    .unwrap();
    let sb = &v["sandbox"];
    assert!(sb["enabled"].as_bool().unwrap());
    assert!(sb["failIfUnavailable"].as_bool().unwrap());
    assert!(!sb["allowUnsandboxedCommands"].as_bool().unwrap());
    assert_eq!(sb["filesystem"]["allowWrite"][0], "/run/wt");
    assert_eq!(sb["filesystem"]["denyWrite"][0], "/orig/a.txt");
    let deny = v["permissions"]["deny"].as_array().unwrap();
    assert!(deny.iter().any(|d| d == "Bash(git push*)"));
    assert!(deny.iter().any(|d| d == "Edit(//orig/a.txt)"));
    let hooks = v["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(hooks.len(), 2);
    // without a sandbox the rules and the hooks stay, the sandbox section goes
    let v: serde_json::Value =
        serde_json::from_str(&barrier::settings(&b, &launch::DENY, &[], false)).unwrap();
    assert!(v.get("sandbox").is_none() && v["permissions"]["deny"].is_array());
}

#[test]
fn a_run_without_a_sandbox_is_refused_unless_the_user_allows_it() {
    let no = cfg_with("");
    let yes = cfg_with("allow_unsandboxed = true");
    assert!(!Config::default().night.allow_unsandboxed);
    assert!(barrier::gate_with(&no, Ok(())).unwrap());
    let why = barrier::gate_with(&no, Err("it needs bwrap".into()))
        .unwrap_err()
        .to_string();
    assert!(
        why.contains("it needs bwrap") && why.contains("allow_unsandboxed"),
        "{why}"
    );
    assert!(!barrier::gate_with(&yes, Err("it needs bwrap".into())).unwrap());
    assert!(barrier::gate_with(&yes, Ok(())).unwrap());
}

#[test]
fn the_claude_command_carries_the_barrier_settings_and_codex_does_not() {
    let cfg = Config::default();
    let s = launch::build(
        &cfg,
        &run_of("claude", "session"),
        "claude",
        Path::new("/w"),
        Some("n"),
        Some("{\"sandbox\":{}}"),
    );
    assert_eq!(after(&s.args, "--settings"), "{\"sandbox\":{}}");
    assert!(after(&s.args, "--disallowedTools").contains("git push"));
    let c = launch::build(
        &cfg,
        &run_of("codex", "session"),
        "codex",
        Path::new("/w"),
        None,
        Some("{}"),
    );
    assert!(!c.args.contains(&"--settings".to_string()));
}

#[test]
fn git_acts_do_not_wait_for_the_repos_hooks_and_never_page() {
    use crate::loose::Probe;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("r");
    repo(&dir);
    git(&dir, &["branch", "night/gone"]);
    // a hook that stalls (an earlier DISCARD printed "git did not answer in time")
    let hook = dir.join(".git/hooks/reference-transaction");
    std::fs::write(&hook, "#!/bin/sh\nsleep 20\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let t = std::time::Instant::now();
    let out = System::new().git_act(&dir, &["branch", "-D", "night/gone"]);
    assert!(out.is_ok(), "{out:?}");
    assert!(t.elapsed().as_secs() < 10, "took {:?}", t.elapsed());
    let branches = git(&dir, &["branch", "--list", "night/*"]);
    assert!(branches.is_empty(), "{branches}");
}

#[test]
fn the_network_of_a_run_is_the_registries_and_no_git_host() {
    let b = barrier::Barrier::default();
    let net = Config::default().night.network_list();
    let v: serde_json::Value =
        serde_json::from_str(&barrier::settings(&b, &launch::DENY, &net, true)).unwrap();
    let n = &v["sandbox"]["network"];
    let allowed: Vec<&str> = n["allowedDomains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    for h in [
        "registry.npmjs.org",
        "pypi.org",
        "crates.io",
        "proxy.golang.org",
    ] {
        assert!(allowed.contains(&h), "{h}");
    }
    for h in ["github.com", "gitlab.com", "bitbucket.org"] {
        assert!(!allowed.contains(&h), "{h}");
        assert!(n["deniedDomains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d == h));
    }
    assert!(n["strictAllowlist"].as_bool().unwrap());
    // [night] network replaces the list
    let cfg = Config::parse("[night]\nnetwork = [\"example.org\"]\n").unwrap();
    assert_eq!(cfg.night.network_list(), vec!["example.org".to_string()]);
    // no sandbox, no network section
    let v: serde_json::Value =
        serde_json::from_str(&barrier::settings(&b, &launch::DENY, &net, false)).unwrap();
    assert!(v.get("sandbox").is_none());
}

#[test]
fn publishing_and_remote_tools_are_denied() {
    for r in [
        "Bash(npm publish*)",
        "Bash(cargo publish*)",
        "Bash(twine upload*)",
        "Bash(gem push*)",
        "Bash(docker push*)",
        "Bash(gh*)",
        "Bash(ssh*)",
        "Bash(scp*)",
        "Bash(git push*)",
    ] {
        assert!(launch::DENY.contains(&r), "{r}");
    }
}

#[test]
fn accept_edits_allows_only_build_and_test_commands() {
    let cfg = Config::parse("[night]\nclaude_permission = \"acceptEdits\"\n").unwrap();
    let s = launch::build(
        &cfg,
        &run_of("claude", "idea"),
        "claude",
        Path::new("/w"),
        None,
        None,
    );
    let allow = after(&s.args, "--allowedTools");
    for broad in [
        "Bash(npm *)",
        "Bash(cargo *)",
        "Bash(npx *)",
        "Bash(python*)",
        "Bash(go *)",
        "Bash(make*)",
    ] {
        assert!(!allow.split(',').any(|a| a == broad), "{broad}");
    }
    for ok in [
        "Bash(npm test*)",
        "Bash(cargo test*)",
        "Bash(python -m pytest*)",
        "Bash(go test*)",
        "Bash(make test*)",
    ] {
        assert!(allow.split(',').any(|a| a == ok), "{ok}");
    }
}

#[test]
fn two_starters_of_one_run_launch_it_once() {
    let db = db_with(1, "2026-11-21");
    let sys = Fake::new("2026-11-21", "02:00");
    let run = super::get(&db, 1).unwrap().unwrap();
    assert!(sched::start_run(&db, &sys, run.clone(), false).unwrap());
    // the second starter still holds the row it read while the run was planned
    assert!(!sched::start_run(&db, &sys, run, false).unwrap());
    assert_eq!(sys.started.borrow().len(), 1);
    assert_eq!(super::get(&db, 1).unwrap().unwrap().status, "running");
}

#[test]
fn a_run_being_launched_cannot_be_cancelled_and_cancel_wins_before() {
    let db = db_with(2, "2026-11-21");
    // a starter has claimed run 1 and is launching it
    db.conn
        .execute("UPDATE night_run SET status='starting' WHERE id=1", [])
        .unwrap();
    let err = super::cancel(&db, 1).unwrap_err().to_string();
    assert!(err.contains("starting"), "{err}");
    assert_eq!(super::get(&db, 1).unwrap().unwrap().status, "starting");
    // a cancel that came first: the starter finds the run no longer planned and launches nothing
    let stale = super::get(&db, 2).unwrap().unwrap();
    super::cancel(&db, 2).unwrap();
    let sys = Fake::new("2026-11-21", "02:00");
    assert!(!sched::start_run(&db, &sys, stale, false).unwrap());
    assert!(sys.started.borrow().is_empty());
    assert_eq!(super::get(&db, 2).unwrap().unwrap().status, "skipped");
}

#[test]
fn a_run_without_a_process_start_time_is_lost_and_never_signalled() {
    let mut db = db_with(1, "2026-11-21");
    let sys = Fake::new("2026-11-21", "02:00");
    sys.no_start_time.set(true);
    let run = super::get(&db, 1).unwrap().unwrap();
    assert!(!sched::start_run(&db, &sys, run, false).unwrap());
    let r = super::get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "lost");
    assert!(r.pid.is_some() && r.why.as_deref().unwrap().contains("start time"));
    assert!(r.ended_at.is_some());
    // later ticks leave it alone
    sched::tick(&mut db, &sys, &cfg_with("")).unwrap();
    assert!(sys.terminated.borrow().is_empty());
    // a running row without a start time (an older eye) is never signalled when a limit is hit
    db.conn
        .execute(
            "UPDATE night_run SET status='running', ended_at=NULL, limit_kind='minutes', limit_value=1, started_at=1 WHERE id=1",
            [],
        )
        .unwrap();
    sys.alive.borrow_mut().insert(r.pid.unwrap());
    sched::tick(&mut db, &sys, &cfg_with("")).unwrap();
    assert!(sys.terminated.borrow().is_empty());
}
