//! `eye night`: plan, list, run and discard night-shift runs. See `docs/night.md`.

use super::args::NightCmd;
use super::Global;
use crate::night::{self, clock, sched, Limit, PlanOpts, Run};
use crate::util::{fmt_ts, now_ms, printable};
use anyhow::{bail, Context, Result};

pub fn run(g: &Global, sub: Option<NightCmd>) -> Result<()> {
    match sub.unwrap_or(NightCmd::Ls { json: false }) {
        NightCmd::On => switch(g, true),
        NightCmd::Off => switch(g, false),
        NightCmd::Add {
            target,
            prompt,
            limit,
            agent,
            switch,
            isolation,
        } => add(g, &target, prompt, limit, agent, switch, isolation),
        NightCmd::Ls { json } => ls(g, json),
        NightCmd::Cancel { run } => {
            let env = g.open()?;
            night::cancel(&env.db, run)?;
            println!("run {run} cancelled");
            Ok(())
        }
        NightCmd::Log { run } => {
            let env = g.open()?;
            night::get(&env.db, run)?.context("no such run")?;
            print!(
                "{}",
                printable_log(&night::read_log(&env.paths, run, 200_000))
            );
            Ok(())
        }
        NightCmd::Window { window } => {
            let w = clock::Window::parse(&window).map_err(anyhow::Error::msg)?;
            let env = g.open()?;
            crate::config::write_night(
                &env.paths.config_path(),
                "window",
                &format!("{:?}", w.text()),
            )?;
            println!("window: {} ({} minutes)", w.text(), w.length_minutes());
            Ok(())
        }
        NightCmd::Parallel { n } => {
            let range = crate::config::PARALLEL_RANGE;
            anyhow::ensure!(
                range.contains(&n),
                "parallel is a number from {} to {}",
                range.start(),
                range.end()
            );
            let env = g.open()?;
            crate::config::write_night(&env.paths.config_path(), "parallel", &n.to_string())?;
            println!("up to {n} runs at once");
            Ok(())
        }
        NightCmd::RunNow { run, yes, wait } => run_now(g, run, yes, wait),
        NightCmd::Tick => {
            let mut env = g.open()?;
            let sys = sched::RealSys::new(&env.paths, &env.cfg, &env.agents);
            let p = sched::tick(&mut env.db, &sys, &env.cfg)?;
            println!(
                "started {:?}, finished {:?}, stopped {:?}",
                p.started, p.finished, p.stopped
            );
            Ok(())
        }
        NightCmd::Bring { run, yes } => bring(g, run, yes),
        NightCmd::Discard {
            run,
            yes,
            lose_changes,
        } => discard(g, run, yes, lose_changes),
    }
}

fn printable_log(s: &str) -> String {
    s.lines().map(printable).collect::<Vec<_>>().join("\n") + "\n"
}

fn switch(g: &Global, on: bool) -> Result<()> {
    let env = g.open()?;
    crate::config::write_night(&env.paths.config_path(), "enabled", &on.to_string())?;
    if on {
        println!(
            "night shift on: planned runs start between {} when nothing you work in is live.\nThe daemon must be running (`eye install`).",
            env.cfg.night.window.replace('-', " and ")
        );
    } else {
        println!("night shift off: nothing starts by itself (runs that already go finish)");
    }
    Ok(())
}

fn add(
    g: &Global,
    target: &str,
    prompt: Option<String>,
    limit: Option<String>,
    agent: Option<String>,
    switch: bool,
    isolation: Option<String>,
) -> Result<()> {
    let env = g.open()?;
    let opts = PlanOpts {
        agent,
        limit: limit.as_deref().map(Limit::parse).transpose()?,
        switch,
        prompt,
        isolation,
    };
    let r = match target.strip_prefix("idea:") {
        Some(n) => {
            let id: i64 = n
                .trim()
                .parse()
                .with_context(|| format!("'{n}' is not an idea number"))?;
            night::plan_idea(&env.db, &env.cfg, &env.agents, id, &opts)?
        }
        None => night::plan_session(&env.db, &env.cfg, &env.agents, target, &opts)?,
    };
    println!(
        "planned run {} for the night of {} ({}, {})",
        r.id,
        r.night,
        r.agent,
        match r.mode() {
            night::Mode::Worktree => "in a worktree of your repo",
            night::Mode::Copy => "in a separate copy",
        }
    );
    if !env.cfg.night.enabled {
        println!("The night shift is off: `eye night on` lets it start.");
    }
    Ok(())
}

fn what(r: &Run) -> String {
    match (&r.idea_id, &r.session_id) {
        (Some(i), _) => format!("idea:{i}"),
        (_, Some(s)) => format!(
            "session {}",
            crate::util::short_id(s).chars().take(6).collect::<String>()
        ),
        _ => "?".into(),
    }
}

fn ls(g: &Global, json: bool) -> Result<()> {
    let env = g.open()?;
    let runs = night::list(&env.db, false)?;
    let now = now_ms();
    if json {
        let v: Vec<_> = runs.iter().map(|r| r.to_json(now)).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let n = &env.cfg.night;
    println!(
        "night shift {}: window {}, quiet {} min, up to {} at once",
        if n.enabled { "on" } else { "off" },
        n.window,
        n.quiet_minutes,
        n.parallel
    );
    if runs.is_empty() {
        println!("no runs planned: `eye night add idea:<n>` or `eye night add <session> --prompt \"...\"`");
    }
    for r in &runs {
        let lim = r
            .limit()
            .map_or(String::new(), |l| format!("  limit {}", l.text()));
        println!(
            "{:>3}  {:<8} {}  {:<6} {}{lim}",
            r.id,
            r.status,
            r.night,
            r.agent,
            what(r)
        );
        println!(
            "       {}",
            printable(&crate::util::head_chars(&r.prompt, 90))
        );
        if let Some(w) = &r.why {
            println!("       {}", printable(w));
        }
        if let (Some(b), Some(wt)) = (&r.branch, &r.worktree) {
            println!("       {b}  {}", crate::util::home_rel(wt));
        } else if let Some(wt) = &r.worktree {
            println!("       copy in {}", crate::util::home_rel(wt));
        }
        if r.is_repo_copy() {
            println!(
                "       a separate copy: `eye night bring {}` puts its branch in your repo",
                r.id
            );
        }
        if let Some(t) = r.ended_at {
            println!("       ended {}", fmt_ts(t));
        }
    }
    Ok(())
}

fn run_now(g: &Global, id: i64, yes: bool, wait: bool) -> Result<()> {
    if !yes {
        bail!("this starts an agent that edits files in its own worktree, ignoring the window and the quiet rule: add --yes");
    }
    let mut env = g.open()?;
    let r = night::get(&env.db, id)?.context("no such run")?;
    anyhow::ensure!(
        r.status == night::PLANNED,
        "run {id} is {}: only a planned run can start",
        r.status
    );
    let sys = sched::RealSys::new(&env.paths, &env.cfg, &env.agents);
    if !sched::start_run(&env.db, &sys, r, false)? {
        let r = night::get(&env.db, id)?.context("run vanished")?;
        bail!("{}", r.why.unwrap_or_else(|| "could not start".into()));
    }
    let r = night::get(&env.db, id)?.context("run vanished")?;
    println!(
        "run {id} started ({}), working in {}",
        r.agent,
        r.worktree
            .as_deref()
            .map_or("?".into(), crate::util::home_rel)
    );
    if !wait {
        println!("the daemon finishes it; `eye night ls` shows how it went");
        return Ok(());
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        sched::tick_with(&mut env.db, &sys, &env.cfg, false)?;
        let r = night::get(&env.db, id)?.context("run vanished")?;
        if r.status != night::RUNNING && !(r.status == night::STOPPED && r.ended_at.is_none()) {
            println!(
                "run {id}: {}{}",
                r.status,
                r.why.map_or(String::new(), |w| format!(" ({w})"))
            );
            if let Some(s) = &r.run_session_id {
                println!("session {s}");
            }
            if let Some(d) = r.diff_stat.as_deref().filter(|d| !d.is_empty()) {
                println!("{}", printable_log(d));
            }
            if let Some(t) = &r.report {
                println!("report:\n{}", printable_log(t));
            }
            return Ok(());
        }
    }
}

fn bring(g: &Global, id: i64, yes: bool) -> Result<()> {
    let env = g.open()?;
    let r = night::get(&env.db, id)?.context("no such run")?;
    anyhow::ensure!(
        r.is_repo_copy(),
        "run {id} is not the copy of a git repo: there is no branch to bring"
    );
    if !yes {
        let q = format!(
            "Fetch the branch {} from the copy into {}?",
            r.branch.as_deref().unwrap_or("?"),
            r.folder
        );
        if !super::ask::confirm(&q, super::ask::From::Terminal("go on without asking"))? {
            println!("left as it is");
            return Ok(());
        }
    }
    println!("{}", night::iso::bring(&crate::loose::System::new(), &r)?);
    Ok(())
}

fn discard(g: &Global, id: i64, yes: bool, lose: bool) -> Result<()> {
    let env = g.open()?;
    let r = night::get(&env.db, id)?.context("no such run")?;
    if !yes {
        let q = format!(
            "Remove {} and its branch? Commits only on that branch are lost.",
            r.worktree.as_deref().unwrap_or("the run's folder")
        );
        if !super::ask::confirm(&q, super::ask::From::Terminal("go on without asking"))? {
            println!("left as it is");
            return Ok(());
        }
    }
    let said = night::iso::discard(
        &crate::loose::System::new(),
        &env.paths.home.join("night"),
        &r,
        lose,
    )?;
    night::discarded(&env.db, &r)?;
    println!("{said}");
    Ok(())
}
