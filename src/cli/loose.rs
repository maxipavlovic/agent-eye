//! `eye loose`: what sessions left behind, and acting on one item after asking.

use super::args::LooseCmd;
use super::Global;
use crate::loose::{self, LooseEnd, System};
use crate::resume;
use crate::util::{now_ms, printable, short_id};
use anyhow::{Context, Result};

pub fn run(g: &Global, sub: Option<LooseCmd>, session: Option<&str>, json: bool) -> Result<()> {
    match sub {
        None => list(g, session, json),
        Some(LooseCmd::Stop { key, yes }) => act(g, &key, yes, Act::Stop),
        Some(LooseCmd::Remove { key, yes }) => act(g, &key, yes, Act::Remove),
        Some(LooseCmd::Ignore { key }) => {
            let env = g.open()?;
            loose::ignore(&env.db, &key, now_ms())?;
            println!("ignored; it is not listed again");
            Ok(())
        }
        Some(LooseCmd::Send {
            session,
            yes,
            dry_run,
        }) => send(g, &session, yes, dry_run),
    }
}

/// A fresh scan in the foreground, stored, then the list.
fn list(g: &Global, session: Option<&str>, json: bool) -> Result<()> {
    let env = g.open()?;
    if !env.cfg.loose.enabled {
        println!("loose ends are off ([loose] enabled = false in config.toml)");
        return Ok(());
    }
    loose::scan_and_store(&env.db, &env.cfg, &env.agents, now_ms())?;
    let sid = session.map(|s| env.db.resolve_id(s)).transpose()?;
    let items = loose::list(&env.db, sid.as_deref(), false)?;
    if json {
        let v: Vec<_> = items.iter().map(LooseEnd::to_json).collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if items.is_empty() {
        println!("nothing left behind");
        return Ok(());
    }
    for i in &items {
        println!(
            "{}  {:<11} {}",
            short_id(&i.session_id).chars().take(6).collect::<String>(),
            i.kind.as_str(),
            printable(&i.line)
        );
        println!("        {}", printable(&i.key));
    }
    if let Some(s) = loose::summary_line(&items) {
        println!("\n{s}");
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    super::ask::confirm(question, super::ask::From::Terminal("go on without asking"))
}

#[derive(Clone, Copy)]
enum Act {
    Stop,
    Remove,
}

fn act(g: &Global, key: &str, yes: bool, what: Act) -> Result<()> {
    let env = g.open()?;
    // the key comes from the last scan; look again when this one is not stored yet
    if loose::get(&env.db, key)?.is_none() {
        loose::scan_and_store(&env.db, &env.cfg, &env.agents, now_ms())?;
    }
    let item = loose::get(&env.db, key)?.with_context(|| format!("no such loose end: {key}"))?;
    let question = item
        .confirm()
        .with_context(|| format!("eye does not act on a {}", item.kind.as_str()))?;
    if !yes && !confirm(&question)? {
        println!("nothing was changed");
        return Ok(());
    }
    let probe = System::with_roots(env.agents.scratch_roots());
    let said = match what {
        Act::Stop => loose::stop(&env.db, &probe, key)?,
        Act::Remove => loose::remove(&env.db, &probe, key)?,
    };
    println!("{said}");
    Ok(())
}

fn send(g: &Global, session: &str, yes: bool, dry_run: bool) -> Result<()> {
    let env = g.open()?;
    let id = env.db.resolve_id(session)?;
    loose::scan_and_store(&env.db, &env.cfg, &env.agents, now_ms())?;
    let items = loose::list(&env.db, Some(&id), false)?;
    if items.is_empty() {
        println!("nothing left behind");
        return Ok(());
    }
    let s = env.db.session_by_id(&id)?.context("session vanished")?;
    let line = env
        .agents
        .resume_line_with(&s, &loose::send_prompt(&items))
        .context("this session has no resume command")?;
    println!("{line}");
    if dry_run {
        return Ok(());
    }
    if !yes && !confirm("Open the session with that message?")? {
        println!("nothing was changed");
        return Ok(());
    }
    crate::archive::restore_if_missing(&env.db, &env.paths, &env.agents, &s)?;
    let plan = resume::plan_command(line, &resume::target(&env.cfg, &env.paths));
    println!("{}", resume::execute(&plan)?);
    Ok(())
}
