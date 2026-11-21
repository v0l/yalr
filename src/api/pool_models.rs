use std::collections::BTreeMap;

use serde::Serialize;

use crate::api::models::{ModelEntry, ModelPricing};
use crate::api::modalities::{provider_architecture, ModelArchitecture};
use crate::db::ModelPricingRow;
use crate::payments::pricing::ResolvedPricing;
use crate::state::AppState;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct PriceRange {
    pub currency: String,
    pub min_input: i64,
    pub max_input: i64,
    pub min_output: i64,
    pub max_output: i64,
    pub unit: String,
}

pub fn price_ranges(options: &[crate::payments::pricing::ResolvedPricing]) -> Vec<PriceRange> {
    let mut by_currency: BTreeMap<&str, PriceRange> = BTreeMap::new();
    for p in options.iter().filter(|p| !p.is_free) {
        by_currency
            .entry(p.currency.as_str())
            .and_modify(|r| {
                r.min_input = r.min_input.min(p.price_per_1m_input_sats);
                r.max_input = r.max_input.max(p.price_per_1m_input_sats);
                r.min_output = r.min_output.min(p.price_per_1m_output_sats);
                r.max_output = r.max_output.max(p.price_per_1m_output_sats);
            })
            .or_insert_with(|| PriceRange {
                currency: p.currency.clone(),
                min_input: p.price_per_1m_input_sats,
                max_input: p.price_per_1m_input_sats,
                min_output: p.price_per_1m_output_sats,
                max_output: p.price_per_1m_output_sats,
                unit: if p.is_sats() { "sats per 1M tokens".into() } else { format!("{} millis per 1M tokens", p.currency) },
            });
    }
    by_currency.into_values().collect()
}

pub async fn owned_slugs(state: &AppState) -> std::collections::HashSet<String> {
    state
        .db
        .list_providers()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.owner_user_id.is_some())
        .map(|p| p.slug)
        .collect()
}

fn cheapest(options: &[ResolvedPricing]) -> Option<ModelPricing> {
    options
        .iter()
        .filter(|p| p.is_sats() && p.is_advertised && !p.is_free)
        .min_by_key(|p| (p.price_per_1m_input_sats + p.price_per_1m_output_sats, p.price_per_request_sats))
        .map(|p| ModelPricing {
            prompt: p.price_per_1m_input_sats,
            completion: p.price_per_1m_output_sats,
            cache_read: p.price_per_1m_cache_read_sats,
            cache_write: p.price_per_1m_cache_write_sats,
            request: p.price_per_request_sats,
            unit: "1M tokens".to_string(),
        })
}

async fn pool_architecture(state: &AppState, model: &str, slugs: &[&str]) -> Option<ModelArchitecture> {
    let mut merged: Option<ModelArchitecture> = None;
    for provider in state.config.router.get_providers().await {
        if !slugs.contains(&provider.slug()) {
            continue;
        }
        let Some(found) = provider_architecture(state, &provider, model).await else {
            continue;
        };
        let all = merged.get_or_insert(ModelArchitecture { input_modalities: Vec::new(), output_modalities: Vec::new() });
        for (into, more) in [(&mut all.input_modalities, found.input_modalities), (&mut all.output_modalities, found.output_modalities)] {
            for modality in more {
                if !into.contains(&modality) {
                    into.push(modality);
                }
            }
        }
    }
    merged
}

pub async fn pool_entries(
    state: &AppState,
    allowed: impl Fn(&str) -> bool,
) -> Vec<ModelEntry> {
    let mut entries = Vec::new();
    for model in state.config.router.full_name_models().await {
        if !allowed(&model) {
            continue;
        }
        if let Some(entry) = pool_entry(state, &model).await {
            entries.push(entry);
        }
    }
    entries
}

pub async fn pool_entry(state: &AppState, model: &str) -> Option<ModelEntry> {
    let router = &state.config.router;
    let members = router.full_name_providers(model).await;
    if members.is_empty() {
        return None;
    }
    let options = match state.payments_state.as_ref() {
        Some(ps) => ps.pricing_resolver.options(model).await,
        None => Vec::new(),
    };
    let ranges = price_ranges(&options);
    let member_ids: Vec<i64> = members.iter().map(|(_, id, _)| *id).collect();
    let rows: Vec<ModelPricingRow> = state
        .db
        .list_model_pricings_by_name(model)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.is_advertised && r.provider_id.is_some_and(|id| member_ids.contains(&id)))
        .collect();
    let slugs: Vec<&str> = members.iter().map(|(slug, _, _)| slug.as_str()).collect();
    Some(ModelEntry {
        id: model.to_string(),
        object: "model".to_string(),
        created: rows.iter().map(|r| r.created_at.timestamp()).min().unwrap_or(0),
        owned_by: "pool".to_string(),
        pricing: cheapest(&options),
        context_length: rows.iter().filter_map(|r| r.context_window).max(),
        max_output_tokens: rows.iter().filter_map(|r| r.max_output_tokens).max(),
        architecture: pool_architecture(state, model, &slugs).await,
        provider_count: Some(members.len()),
        price_range: (!ranges.is_empty()).then_some(ranges),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payments::pricing::ResolvedPricing;

    fn price(currency: &str, input: i64, output: i64) -> ResolvedPricing {
        ResolvedPricing {
            model_name: "m".into(),
            is_free: false,
            is_advertised: true,
            currency: currency.into(),
            owner_user_id: None,
            price_per_1m_input_sats: input,
            price_per_1m_output_sats: output,
            price_per_1m_cache_read_sats: input,
            price_per_1m_cache_write_sats: input,
            price_per_request_sats: 0,
            context_window: 0,
            max_output_tokens: 0,
        }
    }

    #[test]
    fn ranges_are_per_currency_with_min_and_max() {
        let ranges = price_ranges(&[price("usd", 300, 900), price("usd", 100, 1200), price("sats", 5, 6)]);
        assert_eq!(ranges.len(), 2);
        let usd = ranges.iter().find(|r| r.currency == "usd").unwrap();
        assert_eq!((usd.min_input, usd.max_input, usd.min_output, usd.max_output), (100, 300, 900, 1200));
        let sats = ranges.iter().find(|r| r.currency == "sats").unwrap();
        assert_eq!((sats.min_input, sats.max_input), (5, 5));
    }
}
