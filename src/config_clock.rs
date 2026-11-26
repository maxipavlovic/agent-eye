//! `[clock]`: the steering clock, which asks whether the user steers worse at some hours.

use crate::config::Config;
use serde::Deserialize;

/// The shortest and longest `break_minutes`.
pub const BREAK_RANGE: std::ops::RangeInclusive<u32> = 10..=240;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ClockCfg {
    /// Record turn timing and show the clock.
    pub enabled: bool,
    /// Let a model judge which prompts were corrections. Not set: follows `[summary] enabled`,
    /// because both send prompt text to the same agent.
    pub llm: Option<bool>,
    /// Show a hint while the user is in an hour or after a stretch that has gone badly.
    pub hints: bool,
    /// A gap of this many minutes between prompts is a break (10 to 240; other values are
    /// brought into that range when the file is read).
    pub break_minutes: u32,
    /// Take breaks from the time the machine was idle (a few seconds' reading of the system's idle
    /// time, nothing else), instead of from gaps between prompts.
    pub idle: bool,
}

impl Default for ClockCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            llm: None,
            hints: false,
            break_minutes: 45,
            idle: false,
        }
    }
}

impl ClockCfg {
    /// The same settings with `break_minutes` inside its range.
    pub(crate) fn normalized(mut self) -> Self {
        self.break_minutes = self
            .break_minutes
            .clamp(*BREAK_RANGE.start(), *BREAK_RANGE.end());
        self
    }

    /// Does a model judge corrections? `[clock] llm` when it is set, `[summary] enabled` when not.
    pub fn llm_on(cfg: &Config) -> bool {
        cfg.clock.enabled && cfg.clock.llm.unwrap_or(cfg.summary.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_without_clock_gets_the_defaults() {
        let c = Config::parse("[board]\nport = 7500").unwrap().clock;
        assert!(c.enabled && !c.hints && !c.idle);
        assert_eq!((c.llm, c.break_minutes), (None, 45));
    }

    #[test]
    fn the_default_file_loads_with_the_same_defaults() {
        let c = Config::parse(&crate::config::default_toml()).unwrap().clock;
        assert!(c.enabled && !c.hints && !c.idle);
        assert_eq!((c.llm, c.break_minutes), (None, 45));
    }

    #[test]
    fn break_minutes_is_brought_into_range() {
        let parse = |n: u32| {
            Config::parse(&format!("[clock]\nbreak_minutes = {n}"))
                .unwrap()
                .clock
                .break_minutes
        };
        assert_eq!(
            (parse(1), parse(10), parse(90), parse(240), parse(999)),
            (10, 10, 90, 240, 240)
        );
    }

    #[test]
    fn llm_follows_the_summary_setting_until_it_is_set() {
        let on = |t: &str| ClockCfg::llm_on(&Config::parse(t).unwrap());
        assert!(on(""));
        assert!(!on("[summary]\nenabled = false"));
        assert!(on("[summary]\nenabled = false\n[clock]\nllm = true"));
        assert!(!on("[clock]\nllm = false"));
        assert!(!on("[clock]\nenabled = false"));
    }

    #[test]
    fn clock_keys_are_known() {
        let (_, unknown) = Config::parse_noting(
            "[clock]\nenabled = true\nllm = true\nhints = true\nbreak_minutes = 30\nidle = true",
        )
        .unwrap();
        assert!(unknown.is_empty(), "{unknown:?}");
    }
}
