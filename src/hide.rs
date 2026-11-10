//! Throw-away sessions ("/clear", "exit", "cd ..") are hidden from the board, archive, search
//! and listings. A manual `eye hide` / `eye unhide` beats the rule.
use crate::agents::LEGACY_ID;
use crate::config::{Config, HideCfg};
use crate::db::Db;
use crate::model::{Idea, Session};
use anyhow::Result;
use std::collections::HashSet;

/// A single word this short ("exirt", "usage") is a typo or a command, not a task.
const SHORT_WORD_CHARS: usize = 10;

/// With this few prompts the first and last are all of them, so two commands mean nothing else was asked.
const COMMAND_ONLY_MAX_PROMPTS: i64 = 2;

fn is_command(cfg: &HideCfg, text: &str) -> bool {
    let word = text.split_whitespace().next().unwrap_or("");
    text.starts_with('/') || cfg.commands.iter().any(|c| c.eq_ignore_ascii_case(word))
}

/// The rule alone, ignoring manual overrides.
pub fn is_trivial(cfg: &HideCfg, s: &Session, has_outputs: bool, has_children: bool) -> bool {
    if !cfg.trivial
        || has_outputs
        || has_children
        || s.prompt_count > COMMAND_ONLY_MAX_PROMPTS.max(cfg.max_prompts)
    {
        return false;
    }
    // A session that is working right now is not garbage, whatever its first word was.
    if s.live_status.as_deref() == Some("busy") {
        return false;
    }
    let reply = s.last_assistant_text.as_deref().unwrap_or("").trim();
    if reply.chars().count() > cfg.max_reply_chars {
        return false;
    }
    let first = s.first_prompt.as_deref().unwrap_or("").trim();
    if first.is_empty() {
        return false;
    }
    let last = s.last_user_text.as_deref().unwrap_or("").trim();
    if is_command(cfg, first) && is_command(cfg, last) {
        return true;
    }
    if s.prompt_count > cfg.max_prompts {
        return false;
    }
    let word = first.split_whitespace().next().unwrap_or("");
    let single = first.split_whitespace().count() == 1;
    is_command(cfg, first) || (single && word.chars().count() <= SHORT_WORD_CHARS)
}

/// Is the session hidden: by hand, else by the trivial rule?
pub fn is_hidden(cfg: &HideCfg, s: &Session, has_outputs: bool, has_children: bool) -> bool {
    match s.hidden {
        Some(v) => v,
        None => is_trivial(cfg, s, has_outputs, has_children),
    }
}

/// A session of an agent that is switched off (`[agents] enabled`) is left out of every list. Its
/// rows stay in the database and come back when the agent is enabled again.
pub fn agent_off(enabled: &[String], s: &Session) -> bool {
    let id = if s.agent.is_empty() {
        LEGACY_ID
    } else {
        &s.agent
    };
    !enabled.iter().any(|a| a == id)
}

/// The same for an idea; an idea with no agent set belongs to the default agent and stays.
pub fn idea_off(enabled: &[String], i: &Idea) -> bool {
    i.agent
        .as_deref()
        .is_some_and(|a| !a.is_empty() && !enabled.iter().any(|e| e == a))
}

/// Ids of every hidden session: trivial or hidden by hand, or of an agent that is switched off.
pub fn hidden_ids(db: &Db, cfg: &Config) -> Result<HashSet<String>> {
    let outs = db.session_ids_with_outputs()?;
    let kids = db.session_ids_with_children()?;
    let enabled = cfg.enabled_agent_ids();
    Ok(db
        .all_sessions()?
        .iter()
        .filter(|s| {
            agent_off(&enabled, s)
                || is_hidden(&cfg.hide, s, outs.contains(&s.id), kids.contains(&s.id))
        })
        .map(|s| s.id.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_of_a_switched_off_agent_are_hidden() {
        let on = vec!["claude".to_string()];
        let off: Vec<String> = vec![];
        let claude = Session {
            agent: "claude".into(),
            ..Default::default()
        };
        let other = Session {
            agent: "other".into(),
            ..Default::default()
        };
        assert!(!agent_off(&on, &claude) && agent_off(&on, &other) && agent_off(&off, &claude));
        // rows from before the agent column count as Claude Code's
        assert!(!agent_off(&on, &Session::default()));
        let idea = |a: Option<&str>| Idea {
            agent: a.map(String::from),
            ..Default::default()
        };
        assert!(!idea_off(&on, &idea(None)) && !idea_off(&on, &idea(Some("claude"))));
        assert!(idea_off(&on, &idea(Some("other"))));
    }

    fn s(first: &str, prompts: i64, reply: &str) -> Session {
        Session {
            id: "x".into(),
            first_prompt: Some(first.into()),
            prompt_count: prompts,
            last_assistant_text: Some(reply.into()),
            ..Default::default()
        }
    }
    fn trivial(x: &Session, outs: bool, kids: bool) -> bool {
        is_trivial(&HideCfg::default(), x, outs, kids)
    }

    #[test]
    fn real_garbage_is_trivial() {
        for p in [
            "/effort", "/clear", "exirt", "exitr", "cd ..", "resume", "usage",
        ] {
            assert!(trivial(&s(p, 1, "Done."), false, false), "{p}");
        }
        assert!(trivial(&s("/effort", 1, ""), false, false));
        let mut none = s("/clear", 1, "");
        none.last_assistant_text = None;
        assert!(trivial(&none, false, false));
        assert!(trivial(&s("Exit now please", 1, "ok"), false, false));
    }

    #[test]
    fn real_work_stays_visible() {
        // 63 and 5 outputs
        assert!(!trivial(
            &s("PI6 roadmap for team meeting", 1, "ok"),
            true,
            false
        ));
        assert!(!trivial(
            &s("Navigating many open sessions", 1, "ok"),
            true,
            false
        ));
        // a one-question lookup with a long reply
        let reply = "x".repeat(600);
        assert!(!trivial(
            &s("Find the session with Catalog migration.", 1, &reply),
            false,
            false
        ));
        // long reply even for a command
        assert!(!trivial(&s("/effort", 1, &reply), false, false));
        // a sentence that is not a command
        assert!(!trivial(
            &s("what is the weather", 1, "sunny"),
            false,
            false
        ));
        // more prompts, children, busy
        assert!(!trivial(&s("/clear", 2, "ok"), false, false));
        assert!(!trivial(&s("/clear", 1, "ok"), false, true));
        let mut busy = s("refactor", 1, "");
        busy.live_status = Some("busy".into());
        assert!(!trivial(&busy, false, false));
        assert!(!trivial(&s("", 1, "ok"), false, false));
    }

    #[test]
    fn command_only_sessions_are_trivial() {
        let two = |last: &str, n: i64| {
            let mut x = s("/model", n, "ok");
            x.last_user_text = Some(last.into());
            x
        };
        assert!(trivial(&two("/model", 2), false, false));
        assert!(!trivial(&two("fix the login bug", 2), false, false));
        assert!(!trivial(&two("/model", 3), false, false));
    }

    #[test]
    fn config_controls_the_rule() {
        let x = s("/clear", 1, "ok");
        let c = HideCfg {
            trivial: false,
            ..Default::default()
        };
        assert!(!is_trivial(&c, &x, false, false));
        let c = HideCfg {
            max_reply_chars: 1,
            ..Default::default()
        };
        assert!(!is_trivial(&c, &x, false, false));
        let c = HideCfg {
            max_prompts: 3,
            ..Default::default()
        };
        assert!(is_trivial(&c, &s("/clear", 3, "ok"), false, false));
        let c = HideCfg {
            commands: vec!["deploy".into()],
            ..Default::default()
        };
        assert!(is_trivial(
            &c,
            &s("deploy the thing now please", 1, "ok"),
            false,
            false
        ));
        assert!(!is_trivial(
            &c,
            &s("exit the building now", 1, "ok"),
            false,
            false
        ));
    }

    #[test]
    fn manual_override_beats_the_rule() {
        let c = HideCfg::default();
        let mut x = s("/clear", 1, "ok");
        x.hidden = Some(false);
        assert!(!is_hidden(&c, &x, false, false));
        let mut y = s("a real task with words", 5, "long");
        y.hidden = Some(true);
        assert!(is_hidden(&c, &y, true, true));
        y.hidden = None;
        assert!(!is_hidden(&c, &y, true, true));
    }
}
