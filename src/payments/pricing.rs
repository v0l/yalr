use std::sync::Arc;
use crate::config::DefaultPricingConfig;
use crate::db::{Database, ModelPricingRow, SATS};

#[derive(Debug, Clone)]
pub struct ResolvedPricing {
    pub model_name: String,
    pub is_free: bool,
    pub is_advertised: bool,
    pub currency: String,
    pub owner_user_id: Option<i64>,
    pub price_per_1m_input_sats: i64,
    pub price_per_1m_output_sats: i64,
    pub price_per_1m_cache_read_sats: i64,
    pub price_per_1m_cache_write_sats: i64,
    pub price_per_request_sats: i64,
    pub context_window: i32,
    pub max_output_tokens: i32,
}

impl ResolvedPricing {
    pub fn is_sats(&self) -> bool {
        self.currency == SATS
    }

    pub fn token_divisor(&self) -> i64 {
        if self.is_sats() { 1000 } else { 1_000_000 }
    }

    pub fn request_units(&self) -> i64 {
        if self.is_sats() { self.price_per_request_sats * 1000 } else { self.price_per_request_sats }
    }
}

pub struct PricingResolver {
    db: Arc<Database>,
    defaults: DefaultPricingConfig,
}

impl PricingResolver {
    pub fn new(db: Arc<Database>, defaults: DefaultPricingConfig) -> Self {
        Self { db, defaults }
    }

    pub async fn resolve(&self, model_name: &str) -> ResolvedPricing {
        let row = self.db.get_model_pricing(model_name).await.ok().flatten();
        self.from_row(model_name, row.as_ref())
    }

    pub async fn resolve_served(&self, model_name: &str, provider_name: Option<&str>) -> ResolvedPricing {
        match self.served_provider(provider_name).await {
            Some(provider) => self.resolve_for_provider(model_name, &provider).await,
            None => self.resolve(model_name).await,
        }
    }

    async fn resolve_for_provider(&self, model_name: &str, provider: &crate::db::Provider) -> ResolvedPricing {
        let actual = model_name.split_once('/').map_or(model_name, |(_, m)| m);
        let mut row = self.db.get_provider_pricing(model_name, provider.id).await.ok().flatten();
        if row.is_none() {
            row = self.db.get_provider_pricing(actual, provider.id).await.ok().flatten();
        }
        if row.is_none() {
            let rows = self.db.list_model_pricings_by_name(actual).await.unwrap_or_default();
            row = rows.into_iter().find(|r| r.provider_id == Some(provider.id));
        }
        let mut resolved = match row.as_ref() {
            Some(row) => self.from_row(model_name, Some(row)),
            None => self.resolve(model_name).await,
        };
        resolved.owner_user_id = provider.owner_user_id;
        resolved
    }

    pub async fn options(&self, model_name: &str) -> Vec<ResolvedPricing> {
        let Some((prefix, _)) = model_name.split_once('/') else {
            return vec![self.resolve(model_name).await];
        };
        if let Ok(Some(provider)) = self.db.get_provider_by_slug(prefix).await {
            return vec![self.resolve_for_provider(model_name, &provider).await];
        }
        let rows = self.db.list_model_pricings_by_name(model_name).await.unwrap_or_default();
        let priced: Vec<_> = rows.iter().filter(|r| r.provider_id.is_some()).collect();
        if priced.is_empty() {
            return vec![self.resolve(model_name).await];
        }
        priced.into_iter().map(|r| self.from_row(model_name, Some(r))).collect()
    }

    async fn served_provider(&self, name: Option<&str>) -> Option<crate::db::Provider> {
        self.db.get_provider_by_name(name?).await.ok().flatten()
    }

    fn from_row(&self, model_name: &str, row: Option<&ModelPricingRow>) -> ResolvedPricing {
        let Some(mp) = row else {
            return ResolvedPricing {
                model_name: model_name.to_string(),
                is_free: false,
                is_advertised: true,
                currency: SATS.to_string(),
                owner_user_id: None,
                price_per_1m_input_sats: self.defaults.price_per_1m_input_sats,
                price_per_1m_output_sats: self.defaults.price_per_1m_output_sats,
                price_per_1m_cache_read_sats: self.defaults.price_per_1m_input_sats,
                price_per_1m_cache_write_sats: self.defaults.price_per_1m_input_sats,
                price_per_request_sats: self.defaults.price_per_request_sats,
                context_window: self.defaults.context_window,
                max_output_tokens: self.defaults.max_output_tokens,
            };
        };
        let sats = mp.currency == SATS;
        let pick = |sats_value: Option<i64>, milli: Option<i64>| if sats { sats_value } else { milli };
        let input = pick(mp.price_per_1m_input_sats, mp.price_per_1m_input_milli)
            .unwrap_or(if sats { self.defaults.price_per_1m_input_sats } else { 0 });
        ResolvedPricing {
            model_name: model_name.to_string(),
            is_free: mp.is_free,
            is_advertised: mp.is_advertised,
            currency: mp.currency.clone(),
            owner_user_id: mp.owner_user_id,
            price_per_1m_input_sats: input,
            price_per_1m_output_sats: pick(mp.price_per_1m_output_sats, mp.price_per_1m_output_milli)
                .unwrap_or(if sats { self.defaults.price_per_1m_output_sats } else { 0 }),
            price_per_1m_cache_read_sats: pick(mp.price_per_1m_cache_read_sats, mp.price_per_1m_cache_read_milli)
                .unwrap_or(input),
            price_per_1m_cache_write_sats: pick(mp.price_per_1m_cache_write_sats, mp.price_per_1m_cache_write_milli)
                .unwrap_or(input),
            price_per_request_sats: pick(mp.price_per_request_sats, mp.price_per_request_milli)
                .unwrap_or(if sats { self.defaults.price_per_request_sats } else { 0 }),
            context_window: mp.context_window.unwrap_or(self.defaults.context_window),
            max_output_tokens: mp.max_output_tokens.unwrap_or(self.defaults.max_output_tokens),
        }
    }

    pub async fn list_configured(&self) -> Result<Vec<ModelPricingRow>, sqlx::Error> {
        self.db.list_model_pricings().await
    }
}
