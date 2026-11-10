use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::payments::biller::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ListPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

impl ListPrice {
    const fn anthropic(input: f64, output: f64) -> Self {
        Self {
            input,
            output,
            cache_read: Some(input * 0.1),
            cache_write: Some(input * 1.25),
        }
    }

    pub fn cost_usd(&self, usage: &TokenUsage) -> f64 {
        let cached = (usage.cached_tokens as i64).min(usage.prompt_tokens as i64);
        let written = (usage.cache_write_tokens as i64).min(usage.prompt_tokens as i64 - cached);
        let fresh = usage.prompt_tokens as i64 - cached - written;
        let per_token = |rate: f64, tokens: i64| rate * tokens as f64 / 1_000_000.0;
        per_token(self.input, fresh)
            + per_token(self.cache_read.unwrap_or(self.input), cached)
            + per_token(self.cache_write.unwrap_or(self.input), written)
            + per_token(self.output, usage.completion_tokens as i64)
    }
}

static OVERRIDES: RwLock<Option<HashMap<String, ListPrice>>> = RwLock::new(None);

pub fn set_overrides(prices: HashMap<String, ListPrice>) {
    if let Ok(mut guard) = OVERRIDES.write() {
        *guard = Some(prices);
    }
}

fn normalize(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or(model);
    name.to_lowercase().replace(['.', '_'], "-")
}

const LEGACY_OPUS: ListPrice = ListPrice::anthropic(15.0, 75.0);
const CURRENT_OPUS: ListPrice = ListPrice::anthropic(5.0, 25.0);
const SONNET: ListPrice = ListPrice::anthropic(3.0, 15.0);
const HAIKU_4: ListPrice = ListPrice::anthropic(1.0, 5.0);
const HAIKU_3_5: ListPrice = ListPrice::anthropic(0.8, 4.0);
const HAIKU_3: ListPrice = ListPrice::anthropic(0.25, 1.25);

const BUILT_IN: &[(&[&str], ListPrice)] = &[
    (&["opus-4-1", "opus-4-2025", "opus-4-0", "3-opus", "opus-3"], LEGACY_OPUS),
    (&["opus"], CURRENT_OPUS),
    (&["sonnet"], SONNET),
    (&["haiku-4"], HAIKU_4),
    (&["3-5-haiku", "haiku-3-5"], HAIKU_3_5),
    (&["haiku"], HAIKU_3),
];

fn override_for(model: &str) -> Option<ListPrice> {
    let guard = OVERRIDES.read().ok()?;
    let prices = guard.as_ref()?;
    prices
        .get(model)
        .or_else(|| prices.get(model.rsplit('/').next().unwrap_or(model)))
        .copied()
}

pub fn lookup(model: &str) -> Option<ListPrice> {
    if let Some(price) = override_for(model) {
        return Some(price);
    }
    let name = normalize(model);
    BUILT_IN
        .iter()
        .find(|(needles, _)| needles.iter().any(|n| name.contains(n)))
        .map(|(_, price)| *price)
}

pub fn cost_usd(models: &[&str], usage: &TokenUsage, reported: Option<f64>) -> Option<f64> {
    if let Some(cost) = reported.filter(|c| c.is_finite() && *c >= 0.0) {
        return Some(cost);
    }
    models.iter().find_map(|m| lookup(m)).map(|p| p.cost_usd(usage))
}

pub fn to_micro(cost_usd: f64) -> i64 {
    (cost_usd * 1_000_000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(prompt: u32, completion: u32, cached: u32, written: u32) -> TokenUsage {
        TokenUsage { prompt_tokens: prompt, completion_tokens: completion, cached_tokens: cached, cache_write_tokens: written }
    }

    #[test]
    fn sonnet_prices_each_token_class() {
        let price = lookup("claude-sonnet-4-5-20250929").unwrap();
        let cost = price.cost_usd(&usage(1_000_000 + 1_000_000 + 1_000_000, 1_000_000, 1_000_000, 1_000_000));
        assert!((cost - (3.0 + 0.3 + 3.75 + 15.0)).abs() < 1e-9, "{cost}");
    }

    #[test]
    fn opus_generations_differ() {
        assert_eq!(lookup("claude-opus-4-20250514").unwrap().input, 15.0);
        assert_eq!(lookup("claude-opus-4-1-20250805").unwrap().input, 15.0);
        assert_eq!(lookup("claude-opus-4-5").unwrap().input, 5.0);
        assert_eq!(lookup("claude-opus-4.7").unwrap().output, 25.0);
    }

    #[test]
    fn haiku_generations_differ() {
        assert_eq!(lookup("claude-haiku-4-5").unwrap().input, 1.0);
        assert_eq!(lookup("claude-3-5-haiku-latest").unwrap().input, 0.8);
        assert_eq!(lookup("claude-3-haiku-20240307").unwrap().input, 0.25);
    }

    #[test]
    fn a_provider_prefix_is_ignored() {
        assert_eq!(lookup("anthropic/claude-sonnet-4").unwrap().input, 3.0);
        assert!(lookup("gpt-4o").is_none());
    }

    #[test]
    fn reported_cost_beats_the_table() {
        let u = usage(1000, 1000, 0, 0);
        assert_eq!(cost_usd(&["claude-sonnet-4"], &u, Some(0.5)), Some(0.5));
        assert!(cost_usd(&["claude-sonnet-4"], &u, None).unwrap() < 0.5);
        assert_eq!(cost_usd(&["unknown"], &u, None), None);
    }

    #[test]
    fn overrides_win_over_built_ins() {
        set_overrides(HashMap::from([(
            "claude-override-test".to_string(),
            ListPrice { input: 1.0, output: 2.0, cache_read: None, cache_write: None },
        )]));
        let cost = cost_usd(&["claude-override-test"], &usage(1_000_000, 1_000_000, 0, 0), None).unwrap();
        assert!((cost - 3.0).abs() < 1e-9);
        set_overrides(HashMap::new());
    }
}
