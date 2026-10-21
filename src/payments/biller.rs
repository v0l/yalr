//! Billing engine: admission check and charge finalization.
//!
//! A request is admitted when the balance covers the per-request fee. The real cost is
//! charged from the provider's usage afterwards and may take the balance below zero.

use std::sync::Arc;
use crate::payments::balance::{BalanceService, BalanceError};
use crate::payments::pricing::PricingResolver;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
}

fn ceil_msat(micro_sats: i64) -> i64 {
    (micro_sats + 999) / 1000
}

impl TokenUsage {
    pub fn plain(prompt_tokens: u32, completion_tokens: u32) -> Self {
        Self { prompt_tokens, completion_tokens, ..Default::default() }
    }

    pub fn cost_msat(&self, pricing: &crate::payments::pricing::ResolvedPricing) -> i64 {
        let cached = (self.cached_tokens as i64).min(self.prompt_tokens as i64);
        let written = (self.cache_write_tokens as i64).min(self.prompt_tokens as i64 - cached);
        let fresh = self.prompt_tokens as i64 - cached - written;

        let micro = fresh * pricing.price_per_1m_input_sats
            + cached * pricing.price_per_1m_cache_read_sats
            + written * pricing.price_per_1m_cache_write_sats
            + self.completion_tokens as i64 * pricing.price_per_1m_output_sats;
        (ceil_msat(micro) + pricing.price_per_request_sats * 1000).max(0)
    }
}

/// Handles the billing lifecycle for a single request:
/// admit when the balance is positive → finalize with real usage, which may leave it negative.
pub struct BillingEngine {
    balance_service: Arc<BalanceService>,
    pricing_resolver: Arc<PricingResolver>,
}

#[derive(Debug, Clone)]
pub struct Admission {
    pub request_id: String,
    pub user_id: i64,
    pub model: String,
    pub is_free: bool,
}

impl BillingEngine {
    pub fn new(
        balance_service: Arc<BalanceService>,
        pricing_resolver: Arc<PricingResolver>,
    ) -> Self {
        Self {
            balance_service,
            pricing_resolver,
        }
    }

    pub async fn admit(
        &self,
        user_id: i64,
        model_name: &str,
    ) -> Result<Admission, BillingError> {
        let pricing = self.pricing_resolver.resolve(model_name).await;
        let request_id = format!("req-{}", uuid::Uuid::new_v4().simple());
        let admission = Admission {
            request_id,
            user_id,
            model: model_name.to_string(),
            is_free: pricing.is_free,
        };
        if pricing.is_free {
            return Ok(admission);
        }

        let required = (pricing.price_per_request_sats * 1000).max(1);
        let available = self
            .balance_service
            .get_balance(user_id)
            .await
            .map_err(BillingError::Balance)?;
        if available < required {
            return Err(BillingError::InsufficientFunds { required, available });
        }
        Ok(admission)
    }

    pub async fn finalize_charge(
        &self,
        admission: &Admission,
        usage: TokenUsage,
    ) -> Result<i64, BillingError> {
        if admission.is_free {
            return Ok(0);
        }

        let pricing = self.pricing_resolver.resolve(&admission.model).await;
        let actual_msat = usage.cost_msat(&pricing);
        if actual_msat <= 0 {
            return Ok(0);
        }

        self.balance_service
            .charge(admission.user_id, actual_msat, "usage", &admission.request_id, Some(&admission.model))
            .await
            .map_err(BillingError::Balance)?;

        tracing::info!(
            user_id = admission.user_id,
            model = %admission.model,
            actual_msat,
            prompt_tokens = usage.prompt_tokens,
            completion_tokens = usage.completion_tokens,
            cached_tokens = usage.cached_tokens,
            cache_write_tokens = usage.cache_write_tokens,
            "Charge finalized"
        );

        Ok(actual_msat)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BillingError {
    #[error("Insufficient funds: required {required} msats, available {available} msats")]
    InsufficientFunds { required: i64, available: i64 },

    #[error("Balance error: {0}")]
    Balance(#[from] BalanceError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DefaultPricingConfig;
    use crate::db::Database;

    async fn setup() -> BillingEngine {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        // Create a test user
        sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES ('test', 0, 0)")
            .execute(&db.pool)
            .await
            .unwrap();

        let defaults = DefaultPricingConfig {
            price_per_1m_input_sats: 5,    // 5 sats per 1M input tokens
            price_per_1m_output_sats: 15,  // 15 sats per 1M output tokens
            price_per_request_sats: 1,     // 1 sat per request
            context_window: 8192,
            max_output_tokens: 4096,
        };

        let balance_service = Arc::new(BalanceService::new(db.clone()));
        let pricing_resolver = Arc::new(PricingResolver::new(db, defaults));

        BillingEngine::new(balance_service, pricing_resolver)
    }

    #[tokio::test]
    async fn admit_rejects_an_empty_balance() {
        let engine = setup().await;
        let result = engine.admit(1, "gpt-4").await;
        assert!(matches!(result, Err(BillingError::InsufficientFunds { required: 1000, available: 0 })));
    }

    #[tokio::test]
    async fn admit_rejects_a_negative_balance() {
        let engine = setup().await;
        engine.balance_service.charge(1, 500, "usage", "req-x", None).await.unwrap();
        assert!(matches!(engine.admit(1, "gpt-4").await, Err(BillingError::InsufficientFunds { .. })));
    }

    #[tokio::test]
    async fn admit_does_not_move_the_balance() {
        let engine = setup().await;
        engine.balance_service.credit(1, 5000, "deposit", "t").await.unwrap();
        engine.admit(1, "gpt-4").await.unwrap();
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 5000);
    }

    #[tokio::test]
    async fn finalize_charges_real_usage_only() {
        let engine = setup().await;
        engine.balance_service.credit(1, 5000, "deposit", "t").await.unwrap();
        let admission = engine.admit(1, "gpt-4").await.unwrap();
        let billed = engine.finalize_charge(&admission, TokenUsage::plain(50, 100)).await.unwrap();
        assert_eq!(billed, 1002);
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 5000 - 1002);
    }

    #[tokio::test]
    async fn usage_beyond_the_balance_goes_negative() {
        let engine = setup().await;
        engine.balance_service.credit(1, 2000, "deposit", "t").await.unwrap();
        let admission = engine.admit(1, "gpt-4").await.unwrap();
        let billed = engine.finalize_charge(&admission, TokenUsage::plain(2_000_000_000, 0)).await.unwrap();
        assert_eq!(billed, 10_000_000 + 1000);
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 2000 - 10_001_000);
    }

    #[tokio::test]
    async fn a_deposit_pays_off_a_negative_balance() {
        let engine = setup().await;
        engine.balance_service.charge(1, 3000, "usage", "req-x", None).await.unwrap();
        engine.balance_service.credit(1, 5000, "deposit", "t").await.unwrap();
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 2000);
    }

    fn priced(input: i64, output: i64, read: i64, write: i64, request: i64) -> crate::payments::pricing::ResolvedPricing {
        crate::payments::pricing::ResolvedPricing {
            model_name: "m".into(),
            is_free: false,
            is_advertised: true,
            price_per_1m_input_sats: input,
            price_per_1m_output_sats: output,
            price_per_1m_cache_read_sats: read,
            price_per_1m_cache_write_sats: write,
            price_per_request_sats: request,
            context_window: 8192,
            max_output_tokens: 4096,
        }
    }

    #[test]
    fn cache_reads_and_writes_bill_at_their_own_rates() {
        let p = priced(1_000_000, 4_000_000, 100_000, 1_250_000, 0);
        let usage = TokenUsage { prompt_tokens: 1000, completion_tokens: 100, cached_tokens: 600, cache_write_tokens: 200 };
        let msat = (200 * 1_000_000 + 600 * 100_000 + 200 * 1_250_000 + 100 * 4_000_000) / 1000;
        assert_eq!(usage.cost_msat(&p), msat);
    }

    #[test]
    fn unset_cache_prices_bill_like_plain_input() {
        let p = priced(1000, 4000, 1000, 1000, 1);
        let with_cache = TokenUsage { prompt_tokens: 2_000_000, completion_tokens: 0, cached_tokens: 1_500_000, cache_write_tokens: 250_000 };
        let without = TokenUsage::plain(2_000_000, 0);
        assert_eq!(with_cache.cost_msat(&p), without.cost_msat(&p));
    }

    #[test]
    fn cache_counts_above_the_prompt_are_clamped() {
        let p = priced(1000, 0, 100, 2000, 0);
        let usage = TokenUsage { prompt_tokens: 1_000_000, completion_tokens: 0, cached_tokens: 900_000, cache_write_tokens: 900_000 };
        let cost = usage.cost_msat(&p);
        assert!(cost >= 0);
        assert_eq!(cost, (90 + 200) * 1000);
    }

    #[test]
    fn small_requests_bill_fractions_of_a_sat() {
        let p = priced(2000, 10_000, 110, 3000, 0);
        assert_eq!(TokenUsage::plain(100, 0).cost_msat(&p), 200);
        assert_eq!(TokenUsage::plain(1, 0).cost_msat(&p), 2);
        assert_eq!(TokenUsage::plain(0, 1).cost_msat(&p), 10);
        assert_eq!(TokenUsage::plain(1, 0).cost_msat(&priced(1, 0, 1, 1, 0)), 1);
    }

    #[tokio::test]
    async fn test_free_model_skips_charges() {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES ('test', 0, 0)")
            .execute(&db.pool)
            .await
            .unwrap();

        // Create a free model pricing
        sqlx::query(
            "INSERT INTO model_pricing (model_name, is_free, is_advertised) VALUES ('free-model', 1, 1)"
        )
        .execute(&db.pool)
        .await
        .unwrap();

        let defaults = DefaultPricingConfig::default();
        let balance_service = Arc::new(BalanceService::new(db.clone()));
        let pricing_resolver = Arc::new(PricingResolver::new(db, defaults));
        let engine = BillingEngine::new(balance_service, pricing_resolver);

        let admission = engine.admit(1, "free-model").await.unwrap();
        assert!(admission.is_free);

        let actual = engine.finalize_charge(&admission, TokenUsage::plain(1000, 1000)).await.unwrap();
        assert_eq!(actual, 0);
    }
}
