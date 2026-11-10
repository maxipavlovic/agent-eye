//! Which agents an install follows, and the line of the summary that says so.

use crate::agents::{AgentRef, Agents};
use crate::config::Config;
use crate::report::Row;
use crate::util::show_path;
use anyhow::Result;
use std::path::Path;

/// Which agents an install follows, and what it tells the user about it.
#[derive(Debug)]
pub struct Choice {
    /// The config to install with: `[agents] enabled` filled in when it was not set.
    pub cfg: Config,
    /// The ids to write to `[agents] enabled`, when the install sets it.
    pub write: Option<Vec<String>>,
    pub row: Row,
}

/// The names of agents, `Claude Code, Codex CLI`.
fn names(list: &[&AgentRef]) -> String {
    list.iter().map(|a| a.name()).collect::<Vec<_>>().join(", ")
}

/// `[agents] enabled` for an install. A config that names its agents keeps them (an agent that is
/// on the machine and not named is mentioned once). A config that does not (no config yet, or an
/// older one without the key) gets the agents found among `candidates`, in order, so a machine with
/// only Codex follows Codex and one with both follows both; with none found it stays unset, which
/// means Claude Code as it always did.
pub fn choose_agents(
    mut cfg: Config,
    candidates: &Agents,
    config_path: &Path,
    dry_run: bool,
) -> Choice {
    let found: Vec<&AgentRef> = candidates.iter().filter(|a| a.detect().installed).collect();
    let at = show_path(config_path);
    let mut write = None;
    let text = match cfg.agents.enabled.clone() {
        Some(named) if named.is_empty() => {
            format!("none enabled ([agents] enabled in {at}); no hooks installed")
        }
        Some(named) => {
            let following: Vec<&AgentRef> = candidates
                .iter()
                .filter(|a| named.iter().any(|n| n == a.id()))
                .collect();
            let left: Vec<&AgentRef> = found
                .into_iter()
                .filter(|a| !named.iter().any(|n| n == a.id()))
                .collect();
            let who = if following.is_empty() {
                named.join(", ")
            } else {
                names(&following)
            };
            let mut text = format!("{who} (from {at})");
            if !left.is_empty() {
                let ids: Vec<String> = left.iter().map(|a| format!("\"{}\"", a.id())).collect();
                text.push_str(&format!(
                    "; {} found but not followed: add {} to [agents] enabled to follow {}",
                    names(&left),
                    ids.join(", "),
                    if left.len() == 1 { "it" } else { "them" }
                ));
            }
            text
        }
        None if found.is_empty() => {
            "none found; following Claude Code until you install one (see [agents] in config.toml)"
                .to_string()
        }
        None => {
            let ids: Vec<String> = found.iter().map(|a| a.id().to_string()).collect();
            let which = match found.len() {
                1 => "it",
                2 => "both",
                _ => "all",
            };
            cfg.agents.enabled = Some(ids.clone());
            write = Some(ids);
            if dry_run {
                format!("{} (would enable {which} in {at})", names(&found))
            } else if found.len() == 1 {
                format!("{} (enabled in {at})", names(&found))
            } else {
                format!("{} ({which} enabled in {at})", names(&found))
            }
        }
    };
    Choice {
        cfg,
        write,
        row: Row::new("agents", text),
    }
}

/// Write the choice of [`choose_agents`] to the config file.
pub fn save_choice(choice: &Choice, config_path: &Path) -> Result<()> {
    match &choice.write {
        Some(ids) => crate::config::write_agents_enabled(config_path, ids),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::claude::Claude;
    use std::sync::Arc;

    /// Claude Code and Codex under `t`, each with a folder only when asked for and never a program.
    fn machine(t: &Path, claude: bool, codex: bool) -> Agents {
        let dirs = [(t.join("claude"), claude), (t.join("codex"), codex)];
        for (d, there) in &dirs {
            if *there {
                std::fs::create_dir_all(d).unwrap();
            }
        }
        Agents::from_vec(vec![
            Arc::new(Claude::new(dirs[0].0.clone()).with_executable("/nonexistent/claude")),
            Arc::new(
                crate::agents::codex::Codex::new(dirs[1].0.clone())
                    .with_executable("/nonexistent/codex"),
            ),
        ])
    }

    fn choose(t: &Path, claude: bool, codex: bool, cfg: Config) -> Choice {
        choose_agents(
            cfg,
            &machine(t, claude, codex),
            &t.join("config.toml"),
            false,
        )
    }

    fn says(c: &Choice, at: &Path) -> String {
        c.row.text.replace(&at.display().to_string(), "<config>")
    }

    #[test]
    fn a_machine_with_only_codex_follows_codex() {
        let t = tempfile::tempdir().unwrap();
        let c = choose(t.path(), false, true, Config::default());
        assert_eq!(c.cfg.enabled_agent_ids(), ["codex"]);
        assert_eq!(c.write.as_deref(), Some(&["codex".to_string()][..]));
        assert_eq!(c.row.area, "agents");
        assert_eq!(
            says(&c, &t.path().join("config.toml")),
            "Codex CLI (enabled in <config>)"
        );
        // with the config written, the next install keeps it
        save_choice(&c, &t.path().join("config.toml")).unwrap();
        let text = std::fs::read_to_string(t.path().join("config.toml")).unwrap();
        assert!(text.contains("enabled = [\"codex\"]"), "{text}");
        let again = choose(t.path(), false, true, Config::parse(&text).unwrap());
        assert!(again.write.is_none());
        assert_eq!(
            says(&again, &t.path().join("config.toml")),
            "Codex CLI (from <config>)"
        );
    }

    #[test]
    fn a_machine_with_both_follows_both_and_claude_stays_the_default() {
        let t = tempfile::tempdir().unwrap();
        let config = t.path().join("config.toml");
        let c = choose(t.path(), true, true, Config::default());
        assert_eq!(c.cfg.enabled_agent_ids(), ["claude", "codex"]);
        assert_eq!(
            says(&c, &config),
            "Claude Code, Codex CLI (both enabled in <config>)"
        );
        let dry = choose_agents(
            Config::default(),
            &machine(t.path(), true, true),
            &config,
            true,
        );
        assert_eq!(
            says(&dry, &config),
            "Claude Code, Codex CLI (would enable both in <config>)"
        );
        assert!(!config.exists());
    }

    #[test]
    fn a_machine_with_only_claude_gets_what_it_always_had() {
        let t = tempfile::tempdir().unwrap();
        let c = choose(t.path(), true, false, Config::default());
        assert_eq!(c.cfg.enabled_agent_ids(), ["claude"]);
        assert_eq!(c.write.as_deref(), Some(&["claude".to_string()][..]));
        // a config that already names Claude writes nothing and says where it comes from
        let named = Config::parse("[agents]\nenabled = [\"claude\"]\n").unwrap();
        let c = choose(t.path(), true, false, named);
        assert!(c.write.is_none());
        assert_eq!(
            says(&c, &t.path().join("config.toml")),
            "Claude Code (from <config>)"
        );
    }

    #[test]
    fn with_no_agent_found_the_key_stays_unset() {
        let t = tempfile::tempdir().unwrap();
        let c = choose(t.path(), false, false, Config::default());
        assert!(c.write.is_none());
        assert_eq!(c.cfg.agents.enabled, None);
        assert_eq!(c.cfg.enabled_agent_ids(), ["claude"]);
        assert!(
            c.row.text.starts_with("none found; following Claude Code"),
            "{}",
            c.row.text
        );
    }

    #[test]
    fn an_agent_that_is_there_and_not_named_is_mentioned_and_never_switched_on() {
        let t = tempfile::tempdir().unwrap();
        let named = Config::parse("[agents]\nenabled = [\"claude\"]\n").unwrap();
        let c = choose(t.path(), true, true, named);
        assert!(c.write.is_none());
        assert_eq!(c.cfg.enabled_agent_ids(), ["claude"]);
        assert_eq!(
            says(&c, &t.path().join("config.toml")),
            "Claude Code (from <config>); Codex CLI found but not followed: add \"codex\" to [agents] enabled to follow it"
        );
        // somebody who switched every agent off meant it, and is told what that means
        let off = Config::parse("[agents]\nenabled = []\n").unwrap();
        let c = choose(t.path(), true, true, off);
        assert!(c.write.is_none());
        assert!(c.row.text.starts_with("none enabled ("), "{}", c.row.text);
        assert!(c.row.text.ends_with("; no hooks installed"));
    }
}
