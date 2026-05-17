use std::sync::Arc;
use crate::config::DefaultPricingConfig;
use crate::db::{Database, ModelPricingRow};

/// Resolved pricing for a model — merges per-model overrides with defaults.
#[derive(Debug, Clone)]
pub struct ResolvedPricing {
    pub model_name: String,
    pub is_free: bool,
    pub is_advertised: bool,
    pub price_per_1m_input_sats: i64,
    pub price_per_1m_output_sats: i64,
    pub price_per_request_sats: i64,
    pub context_window: i32,
    pub max_output_tokens: i32,
}

/// Resolves pricing by checking the model_pricing table, falling back
/// to config.yaml defaults.
pub struct PricingResolver {
    db: Arc<Database>,
    defaults: DefaultPricingConfig,
}

impl PricingResolver {
    pub fn new(db: Arc<Database>, defaults: DefaultPricingConfig) -> Self {
        Self { db, defaults }
    }

    /// Resolve pricing for a given model name.
    /// Checks model_pricing table first; NULL fields fall through to defaults.
    pub async fn resolve(&self, model_name: &str) -> ResolvedPricing {
        let row = self.db.get_model_pricing(model_name).await.ok().flatten();

        match row {
            Some(mp) => ResolvedPricing {
                model_name: model_name.to_string(),
                is_free: mp.is_free,
                is_advertised: mp.is_advertised,
                price_per_1m_input_sats: mp.price_per_1m_input_sats
                    .unwrap_or(self.defaults.price_per_1m_input_sats),
                price_per_1m_output_sats: mp.price_per_1m_output_sats
                    .unwrap_or(self.defaults.price_per_1m_output_sats),
                price_per_request_sats: mp.price_per_request_sats
                    .unwrap_or(self.defaults.price_per_request_sats),
                context_window: mp.context_window.unwrap_or(self.defaults.context_window),
                max_output_tokens: mp.max_output_tokens.unwrap_or(self.defaults.max_output_tokens),
            },
            None => ResolvedPricing {
                model_name: model_name.to_string(),
                is_free: false,
                is_advertised: true, // implicitly advertised until restricted
                price_per_1m_input_sats: self.defaults.price_per_1m_input_sats,
                price_per_1m_output_sats: self.defaults.price_per_1m_output_sats,
                price_per_request_sats: self.defaults.price_per_request_sats,
                context_window: self.defaults.context_window,
                max_output_tokens: self.defaults.max_output_tokens,
            },
        }
    }

    /// List all explicitly configured model pricings from DB.
    pub async fn list_configured(&self) -> Result<Vec<ModelPricingRow>, sqlx::Error> {
        self.db.list_model_pricings().await
    }
}
