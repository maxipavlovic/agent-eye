//! What the night shift refuses or declines to do: a run that is over its limit before it starts,
//! a limit that rounds to nothing, a switch on words that only look like a rate limit, and an
//! agent that is not allowed to run at night as the one to switch to.

use super::*;

/// The weekly window of `agent` is at `pct` percent and resets in an hour.
fn weekly_at(db: &Db, f: &Fake, agent: &str, pct: f64) {
    crate::usage::limits::record(
        &db.conn,
        agent,
        &crate::usage::PlanLimits {
            seen_at: f.now.get(),
            plan: None,
            windows: vec![weekly_window(pct, Some(f.now.get() + 3_600_000))],
        },
    )
    .unwrap();
}

// ---------- a limit that is already passed ----------

#[test]
fn a_run_already_above_its_weekly_limit_is_refused_before_anything_is_made() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    running_run(&db, &f, &cfg, Some("30%"), "s-1");
    weekly_at(&db, &f, "claude", 35.0);
    let pass = tick(&mut db, &f, &cfg);
    assert!(pass.started.is_empty() && f.started.borrow().is_empty());
    assert!(f.terminated.borrow().is_empty());
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "failed");
    assert_eq!(
        r.why.as_deref(),
        Some("already above its limit: weekly limit 30% reached (35% used)")
    );
    assert!(r.ended_at.is_some() && r.pid.is_none() && r.worktree.is_none());
}

#[test]
fn run_now_refuses_a_run_above_its_limit_too() {
    let cfg = cfg_with("");
    let db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    let run = running_run(&db, &f, &cfg, Some("30%"), "s-1");
    weekly_at(&db, &f, "claude", 35.0);
    assert!(!sched::start_run(&db, &f, run, false).unwrap());
    assert!(f.started.borrow().is_empty());
    assert_eq!(status_of(&db, 1), "failed");
}

#[test]
fn a_run_under_its_weekly_limit_starts() {
    let cfg = cfg_with("");
    let mut db = db_with(1, "2026-11-21");
    let f = Fake::new("2026-11-21", "03:00");
    running_run(&db, &f, &cfg, Some("30%"), "s-1");
    weekly_at(&db, &f, "claude", 29.0);
    assert_eq!(tick(&mut db, &f, &cfg).started, vec![1]);
    assert_eq!(status_of(&db, 1), "running");
}

#[test]
fn a_run_that_moves_to_the_other_agent_is_not_held_to_a_limit_about_the_old_plan() {
    let cfg = two_agents();
    let mut db = db_with(1, "2026-11-21");
    db.conn
        .execute("UPDATE night_run SET switch_ok=1 WHERE id=1", [])
        .unwrap();
    let mut f = Fake::new("2026-11-21", "03:00");
    f.other = Some("codex".into());
    running_run(&db, &f, &cfg, Some("30%"), "s-1");
    weekly_at(&db, &f, "claude", 100.0);
    assert_eq!(tick(&mut db, &f, &cfg).started, vec![1]);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!((r.agent.as_str(), r.status.as_str()), ("codex", "running"));
}

#[test]
fn a_limit_that_rounds_to_nothing_is_refused() {
    for bad in ["0.5m", "0.9m", "0.4%", "0.01h", "0.0004k", "0.0000004M"] {
        let e = Limit::parse(bad).unwrap_err().to_string();
        assert!(e.contains("rounds to 0"), "{bad}: {e}");
    }
    assert_eq!(Limit::parse("1m").unwrap(), Limit::Minutes(1));
    assert_eq!(Limit::parse("1.5m").unwrap(), Limit::Minutes(1));
    assert_eq!(Limit::parse("0.5%").unwrap(), Limit::WeeklyPercent(1));
    assert_eq!(Limit::parse("0.0001M").unwrap(), Limit::Tokens(100));
}

// ---------- a switch needs the agent's own words ----------

#[test]
fn each_agents_own_rate_limit_line_counts() {
    for (agent, line) in [
        // seen in Claude Code's transcripts
        (
            "claude",
            "You've hit your session limit · resets 3pm (Europe/Madrid)",
        ),
        (
            "claude",
            "You've hit your weekly limit · resets Nov 30 at 3pm (Europe/Madrid)",
        ),
        // seen in Antigravity's own files
        (
            "agy",
            "API error (attempt 1): RESOURCE_EXHAUSTED (code 429): You have exhausted your capacity on this model.",
        ),
        // as the CLIs word them, not seen on this machine
        ("claude", "API Error: 429 rate limited"),
        ("codex", "ERROR: You've hit your usage limit. Try again later."),
        (
            "codex",
            "ERROR: exceeded retry limit, last status: 429 Too Many Requests",
        ),
        ("opencode", "Error: Rate limit exceeded"),
    ] {
        assert!(
            sched::rate_limited(agent, &format!("working on it\n{line}\n")),
            "{agent}: {line}"
        );
    }
}

#[test]
fn words_in_a_runs_own_output_are_not_a_rate_limit() {
    for agent in ["claude", "codex", "agy", "opencode"] {
        for text in [
            "tests passed, nothing to see",
            "I added a retry for HTTP 429 and a quota check; the limit reached case is tested",
            "the handler returns too many requests when the rate limit is hit",
            "see quota.md for the usage limit of the plan",
            "assert resp.status == 429",
        ] {
            assert!(!sched::rate_limited(agent, text), "{agent}: {text}");
        }
    }
    // another agent's words say nothing about this one
    assert!(!sched::rate_limited(
        "codex",
        "You've hit your session limit · resets 3pm"
    ));
}

#[test]
fn a_run_that_failed_on_its_own_words_about_quotas_is_not_planned_again() {
    let cfg = two_agents();
    let mut db = db_with(1, "2026-11-21");
    db.conn
        .execute("UPDATE night_run SET switch_ok=1 WHERE id=1", [])
        .unwrap();
    let mut f = Fake::new("2026-11-21", "03:00");
    f.other = Some("codex".into());
    tick(&mut db, &f, &cfg);
    f.tails.borrow_mut().insert(
        1,
        "the quota handler returns 429 too many requests, limit reached\ncould not finish".into(),
    );
    f.end(&get(&db, 1).unwrap().unwrap(), 1);
    tick(&mut db, &f, &cfg);
    let r = get(&db, 1).unwrap().unwrap();
    assert_eq!(r.status, "failed");
    assert!(!r.why.unwrap().contains("started again"));
    assert!(get(&db, 2).unwrap().is_none());
}

// ---------- the agent to switch to has to be allowed to run at night ----------

/// The other agent a real machine would pick for a run of `agent`, with these agents enabled.
fn other_of(agent: &str, config: &str) -> Option<String> {
    let cfg = Config::parse(config).unwrap();
    let agents = crate::agents::enabled(&cfg);
    let paths = Paths {
        home: std::env::temp_dir(),
    };
    sched::RealSys::new(&paths, &cfg, &agents).other_agent(agent)
}

#[test]
fn the_other_agent_is_one_whose_night_gate_passes() {
    // Antigravity and OpenCode need `allow_unsandboxed`: without it they cannot start
    for agents in [
        "[agents]\nenabled = [\"claude\", \"agy\", \"codex\"]\n",
        "[agents]\nenabled = [\"claude\", \"opencode\", \"codex\"]\n",
    ] {
        assert_eq!(other_of("claude", agents).as_deref(), Some("codex"));
    }
    for agents in [
        "[agents]\nenabled = [\"claude\", \"agy\"]\n",
        "[agents]\nenabled = [\"claude\", \"opencode\"]\n",
        "[agents]\nenabled = [\"claude\"]\n",
    ] {
        assert_eq!(other_of("claude", agents), None, "{agents}");
    }
    let open = "[agents]\nenabled = [\"claude\", \"agy\"]\n[night]\nallow_unsandboxed = true\n";
    assert_eq!(other_of("claude", open).as_deref(), Some("agy"));
}
