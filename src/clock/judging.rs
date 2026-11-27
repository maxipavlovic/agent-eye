//! Whether corrections are being judged and by whom: the one place the report, the verdict's text
//! and the pass ask, so they always give the same reason.

use crate::agents::{AgentRef, Agents};
use crate::config::Config;
use crate::config_clock::ClockCfg;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Judging {
    On,
    /// Switched off in the config.
    Off,
    /// On, but there is no agent that can judge.
    Unavailable,
}

/// The agent that judges corrections, or why there is none. It is the summary agent, and only when
/// its headless run is proven locked down: the prompts it reads are the user's own text, which may
/// hold instructions planted by a web page or a file.
pub fn judge<'a>(cfg: &Config, agents: &'a Agents) -> Result<&'a AgentRef, (Judging, String)> {
    if !cfg.clock.enabled {
        return Err((
            Judging::Off,
            "the steering clock is off ([clock] enabled)".into(),
        ));
    }
    if !ClockCfg::llm_on(cfg) {
        let why = match cfg.clock.llm {
            Some(false) => "[clock] llm is off",
            _ => "[summary] enabled is off, and corrections follow it",
        };
        return Err((Judging::Off, why.into()));
    }
    let Some(agent) = agents.summary_agent(cfg) else {
        return Err((
            Judging::Unavailable,
            "no enabled agent can judge corrections".into(),
        ));
    };
    if !agent.judging_locked_down() {
        return Err((
            Judging::Unavailable,
            format!(
                "{} is not used for judging: its locked-down run has not been checked on a real install",
                agent.name()
            ),
        ));
    }
    Ok(agent)
}

#[cfg(test)]
mod tests;
