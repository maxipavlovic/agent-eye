//! List prices per model, in USD per million tokens, for the cost estimate. This is the one place
//! prices live. Checked on 2026-11-21 against the pages below; an unknown model has no price and
//! no cost is shown for it, never a guess. `[usage.prices."<model>"]` in `config.toml` adds a model
//! or overrides one here.
//!
//! The result is an API-equivalent estimate, not a bill: a subscription is a flat fee, and a
//! price list moves. Cache writes are priced as 5-minute writes (Claude Code's default).

use crate::config::{PriceCfg, UsageCfg};

/// Where Claude prices come from (checked 2026-11-21).
pub const CLAUDE_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing";
/// Where OpenAI prices come from (checked 2026-11-21).
pub const OPENAI_SOURCE: &str = "https://developers.openai.com/api/docs/pricing";
/// Where Gemini prices come from (checked 2026-11-21). agy runs Gemini models through Google.
pub const GOOGLE_SOURCE: &str = "https://ai.google.dev/gemini-api/docs/pricing";
/// The date the table was checked.
pub const CHECKED: &str = "2026-11-21";
/// What every cost line says it is.
pub const LABEL: &str = "API-equivalent estimate, not your bill";

/// USD per million tokens.
pub type Price = PriceCfg;

const fn p(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Price {
    PriceCfg {
        input,
        output,
        cache_write,
        cache_read,
    }
}

/// Claude, from the Claude pricing page: base input, output, 5-minute cache write, cache hit.
/// Haiku 5.5 is the price for prompts up to 100,000 tokens (the page prices a longer prompt
/// higher, which the daily totals cannot tell), so its cost is a floor.
const CLAUDE: &[(&str, Price)] = &[
    ("claude-fable-5-2", p(8.0, 40.0, 10.0, 0.20)),
    ("claude-mythos-5-2", p(8.0, 40.0, 10.0, 0.20)),
    ("claude-fable-5-1", p(10.0, 50.0, 12.5, 0.25)),
    ("claude-mythos-5-1", p(10.0, 50.0, 12.5, 0.25)),
    ("claude-fable-5", p(10.0, 50.0, 12.5, 1.0)),
    ("claude-mythos-5", p(10.0, 50.0, 12.5, 1.0)),
    ("claude-opus-5-5", p(4.0, 20.0, 5.0, 0.20)),
    ("claude-opus-5", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-8", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-7", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-6", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-5", p(5.0, 25.0, 6.25, 0.50)),
    ("claude-opus-4-1", p(15.0, 75.0, 18.75, 1.50)),
    ("claude-opus-4", p(15.0, 75.0, 18.75, 1.50)),
    ("claude-sonnet-5-5", p(2.0, 10.0, 2.5, 0.10)),
    ("claude-sonnet-5", p(2.0, 10.0, 2.5, 0.20)),
    ("claude-sonnet-4-6", p(3.0, 15.0, 3.75, 0.30)),
    ("claude-sonnet-4-5", p(3.0, 15.0, 3.75, 0.30)),
    ("claude-sonnet-4", p(3.0, 15.0, 3.75, 0.30)),
    ("claude-haiku-5-5", p(0.10, 0.50, 0.125, 0.01)),
    ("claude-haiku-4-5", p(1.0, 5.0, 1.25, 0.10)),
];

/// OpenAI, from the OpenAI pricing page (standard tier, prompts up to 272K tokens): input, output,
/// no cache write, cached input. Codex counts cached input as part of input; eye splits it out.
const OPENAI: &[(&str, Price)] = &[
    ("gpt-6.2-astra", p(8.0, 40.0, 0.0, 0.40)),
    ("gpt-6.2-sol", p(2.0, 10.0, 0.0, 0.10)),
    ("gpt-6-astra", p(10.0, 50.0, 0.0, 1.0)),
    ("gpt-6.1-sol", p(2.0, 10.0, 0.0, 0.10)),
    ("gpt-6.1-terra", p(1.0, 5.0, 0.0, 0.05)),
    ("gpt-6-luna", p(0.10, 0.50, 0.0, 0.01)),
    ("gpt-6-sol", p(2.0, 10.0, 0.0, 0.20)),
    ("gpt-5.6-sol", p(4.0, 20.0, 0.0, 0.40)),
    ("gpt-5.6-terra", p(2.0, 12.0, 0.0, 0.20)),
    ("gpt-5.6-luna", p(0.20, 1.20, 0.0, 0.02)),
    ("gpt-5.5", p(5.0, 30.0, 0.0, 0.50)),
    ("gpt-5.4-mini", p(0.75, 4.50, 0.0, 0.075)),
    ("gpt-5.4-nano", p(0.20, 1.25, 0.0, 0.02)),
    ("gpt-5.4", p(2.5, 15.0, 0.0, 0.25)),
    ("gpt-5.3-codex", p(1.75, 14.0, 0.0, 0.175)),
    ("gpt-5.2", p(1.75, 14.0, 0.0, 0.175)),
    ("gpt-5.1", p(1.25, 10.0, 0.0, 0.125)),
    ("gpt-5-mini", p(0.25, 2.0, 0.0, 0.025)),
    ("gpt-5-nano", p(0.05, 0.40, 0.0, 0.005)),
    ("gpt-5", p(1.25, 10.0, 0.0, 0.125)),
];

/// Gemini, from the Gemini API pricing page (standard tier, no prompt-size tiers on these models):
/// input, output with thinking, no cache write, cache read. Gemini 3.8 Flash costs $0.75 / $3.75 /
/// $0.075 through 2026-12-31 and double from 2027-01-01 (this table has the price now); agy's
/// High, Medium and Low are reasoning levels of the same model and cost the same. GPT-OSS 120B,
/// which agy also offers, is not on that page and has no price here (`[usage.prices]` adds one).
const GOOGLE: &[(&str, Price)] = &[
    ("gemini-3.8-flash", p(0.75, 3.75, 0.0, 0.075)),
    ("gemini-3.8-flash-high", p(0.75, 3.75, 0.0, 0.075)),
    ("gemini-3.8-flash-medium", p(0.75, 3.75, 0.0, 0.075)),
    ("gemini-3.8-flash-low", p(0.75, 3.75, 0.0, 0.075)),
    ("gemini-3.6-flash", p(0.75, 3.75, 0.0, 0.075)),
    ("gemini-3-flash-preview", p(0.50, 3.0, 0.0, 0.05)),
    ("gemini-3.5-flash-lite", p(0.30, 2.50, 0.0, 0.03)),
    ("gemini-3.1-flash-lite", p(0.25, 1.50, 0.0, 0.025)),
];

/// agy's database names a call by its display name (`Gemini 3.8 Flash (High)`), its log by the key
/// (`gemini-3.8-flash-high`): the display name as the key.
fn gemini_key(m: &str) -> String {
    match m.strip_prefix("gemini ").and_then(|r| r.strip_suffix(')')) {
        Some(r) => match r.split_once(" (") {
            Some((name, level)) => format!("gemini-{}-{level}", name.replace(' ', "-")),
            None => m.to_string(),
        },
        None => m.replace(' ', "-"),
    }
}

/// `claude-opus-4-5-20251101` as `claude-opus-4-5`: lower case, no `[1m]` tag, no date.
fn normalize(model: &str) -> String {
    let m = model.trim().to_ascii_lowercase();
    let m = m.split('[').next().unwrap_or("").trim_end_matches('-');
    let dated = m
        .rsplit_once('-')
        .is_some_and(|(_, d)| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()));
    if dated {
        m.rsplit_once('-').map_or(m, |(a, _)| a).to_string()
    } else {
        m.to_string()
    }
}

/// The price of a model: the config first (by the name as written or normalised), then the
/// built-in table. None for a model that is in neither.
pub fn lookup(model: &str, cfg: &UsageCfg) -> Option<Price> {
    let norm = normalize(model);
    if let Some(p) = cfg
        .prices
        .iter()
        .find(|(k, _)| k.as_str() == model || normalize(k) == norm)
        .map(|(_, p)| *p)
    {
        return Some(p);
    }
    let key = if norm.starts_with("gemini ") {
        gemini_key(&norm)
    } else {
        norm
    };
    CLAUDE
        .iter()
        .chain(OPENAI.iter())
        .chain(GOOGLE.iter())
        .find(|(id, _)| *id == key)
        .map(|(_, p)| *p)
}

/// What `[input, output, cache write, cache read]` tokens cost at `price`, in USD.
pub fn cost(price: &Price, t: [i64; 4]) -> f64 {
    (t[0] as f64 * price.input
        + t[1] as f64 * price.output
        + t[2] as f64 * price.cache_write
        + t[3] as f64 * price.cache_read)
        / 1e6
}

/// The cost of tokens on a model, None when the model has no price.
pub fn cost_of(model: &str, t: [i64; 4], cfg: &UsageCfg) -> Option<f64> {
    lookup(model, cfg).map(|pr| cost(&pr, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_follows_the_four_prices() {
        let pr = lookup("claude-opus-5-5", &UsageCfg::default()).unwrap();
        // 1M in, 1M out, 1M cache write, 1M cache read
        let c = cost(&pr, [1_000_000; 4]);
        assert!((c - (4.0 + 20.0 + 5.0 + 0.20)).abs() < 1e-9, "{c}");
    }

    #[test]
    fn dates_and_context_tags_do_not_change_the_model() {
        let cfg = UsageCfg::default();
        assert_eq!(
            lookup("claude-opus-4-5-20251101", &cfg),
            lookup("claude-opus-4-5", &cfg)
        );
        assert_eq!(
            lookup("Claude-Sonnet-5-5[1m]", &cfg),
            lookup("claude-sonnet-5-5", &cfg)
        );
        assert!(lookup("claude-opus-4-5", &cfg).is_some());
    }

    #[test]
    fn an_unknown_model_has_no_price_and_no_cost() {
        let cfg = UsageCfg::default();
        assert!(lookup("claude-future-9", &cfg).is_none());
        assert!(lookup("<synthetic>", &cfg).is_none());
        assert!(lookup("", &cfg).is_none());
        assert_eq!(cost_of("claude-future-9", [1, 2, 3, 4], &cfg), None);
    }

    #[test]
    fn a_family_is_not_matched_by_prefix() {
        // `claude-opus-5-5` and `claude-opus-5` are different prices; `gpt-5.5-pro` is not listed.
        let cfg = UsageCfg::default();
        assert_ne!(
            lookup("claude-opus-5", &cfg),
            lookup("claude-opus-5-5", &cfg)
        );
        assert!(lookup("gpt-5.5-pro", &cfg).is_none());
    }

    #[test]
    fn the_config_overrides_and_adds_models() {
        let mut cfg = UsageCfg::default();
        cfg.prices
            .insert("claude-opus-5-5".into(), p(1.0, 2.0, 3.0, 4.0));
        cfg.prices.insert("my-model".into(), p(10.0, 0.0, 0.0, 0.0));
        let c = cost_of(
            "claude-opus-5-5",
            [1_000_000, 1_000_000, 1_000_000, 1_000_000],
            &cfg,
        )
        .unwrap();
        assert!((c - 10.0).abs() < 1e-9);
        assert!((cost_of("my-model", [500_000, 9, 9, 9], &cfg).unwrap() - 5.0).abs() < 1e-3);
        assert!(cost_of("other", [1, 1, 1, 1], &cfg).is_none());
    }

    #[test]
    fn the_table_has_no_duplicates() {
        let mut ids: Vec<&str> = CLAUDE
            .iter()
            .chain(OPENAI.iter())
            .chain(GOOGLE.iter())
            .map(|(i, _)| *i)
            .collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(n, ids.len());
    }

    #[test]
    fn gemini_costs_follow_google_prices_by_key_or_display_name() {
        let cfg = UsageCfg::default();
        let by_key = lookup("gemini-3.8-flash-low", &cfg).unwrap();
        assert_eq!(lookup("Gemini 3.8 Flash (High)", &cfg), Some(by_key));
        assert_eq!(lookup("Gemini 3.8 Flash (Low)", &cfg), Some(by_key));
        // 1M in, 1M out, 1M cached: 0.75 + 3.75 + 0.075
        let c = cost_of(
            "gemini-3.8-flash-high",
            [1_000_000, 1_000_000, 0, 1_000_000],
            &cfg,
        )
        .unwrap();
        assert!((c - 4.575).abs() < 1e-9, "{c}");
        let lite = cost_of("gemini-3.5-flash-lite", [2_000_000, 1_000_000, 0, 0], &cfg).unwrap();
        assert!((lite - 3.10).abs() < 1e-9, "{lite}");
        assert!(lookup("GPT-OSS 120B (Medium)", &cfg).is_none());
        assert!(lookup("Gemini 9 Nothing (High)", &cfg).is_none());
    }
}
