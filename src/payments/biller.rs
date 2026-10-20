//! Billing engine — max cost calculation, balance reservation, charge finalization.
//!
//! Follows RIP-05 pricing: max_cost is computed upfront from context window and
//! max output tokens. The actual charge uses real usage data from the provider response.

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
        let total_sats = micro / 1_000_000 + pricing.price_per_request_sats;

        (total_sats * 1000).max(0)
    }
}

/// Handles the billing lifecycle for a single request:
/// resolve pricing → compute max_cost → reserve → finalize with real usage.
pub struct BillingEngine {
    balance_service: Arc<BalanceService>,
    pricing_resolver: Arc<PricingResolver>,
}

/// Result of reserving balance for a request.
#[derive(Debug, Clone)]
pub struct Reservation {
    pub request_id: String,
    pub user_id: i64,
    pub model: String,
    pub reserved_msat: i64,
    pub max_cost_msat: i64,
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

    /// Calculate the maximum possible cost for a request to the given model.
    ///
    /// Formula (RIP-05):
    /// ```text
    /// max_cost = (context_window * input_price_per_1M) / 1_000_000
    ///          + (max_output_tokens * output_price_per_1M) / 1_000_000
    ///          + request_fee
    /// ```
    ///
    /// If `request_max_tokens` is provided, uses `min(context_window, request_max_tokens)`
    /// for the input side.
    pub async fn calculate_max_cost(
        &self,
        model_name: &str,
        request_max_tokens: Option<u32>,
    ) -> Result<i64, BillingError> {
        let pricing = self.pricing_resolver.resolve(model_name).await;

        if pricing.is_free {
            return Ok(0);
        }

        let input_tokens = request_max_tokens
            .map(|t| t.min(pricing.context_window as u32))
            .unwrap_or(pricing.context_window as u32) as i64;

        let output_tokens = pricing.max_output_tokens as i64;

        // (tokens * sats_per_1M) / 1_000_000 * 1000 to get msats
        let input_cost = (input_tokens * pricing.price_per_1m_input_sats) / 1_000_000;
        let output_cost = (output_tokens * pricing.price_per_1m_output_sats) / 1_000_000;
        let request_cost = pricing.price_per_request_sats;

        let total_sats = input_cost + output_cost + request_cost;
        Ok(total_sats * 1000) // convert sats → msats
    }

    /// Reserve balance for a request. Returns a `Reservation` that should be
    /// passed to `finalize_charge` after the request completes.
    pub async fn reserve(
        &self,
        user_id: i64,
        model_name: &str,
        request_max_tokens: Option<u32>,
    ) -> Result<Reservation, BillingError> {
        let max_cost = self.calculate_max_cost(model_name, request_max_tokens).await?;
        let pricing = self.pricing_resolver.resolve(model_name).await;

        let request_id = format!("req-{}", uuid::Uuid::new_v4().simple());

        if pricing.is_free {
            return Ok(Reservation {
                request_id,
                user_id,
                model: model_name.to_string(),
                reserved_msat: 0,
                max_cost_msat: 0,
                is_free: true,
            });
        }

        if max_cost == 0 {
            return Ok(Reservation {
                request_id,
                user_id,
                model: model_name.to_string(),
                reserved_msat: 0,
                max_cost_msat: 0,
                is_free: false,
            });
        }

        self.balance_service
            .debit_with_metadata(user_id, max_cost, "reserve", &request_id, Some(model_name))
            .await
            .map_err(|e| match e {
                BalanceError::InsufficientFunds { required, available } => {
                    BillingError::InsufficientFunds { required, available }
                }
                other => BillingError::Balance(other),
            })?;

        Ok(Reservation {
            request_id,
            user_id,
            model: model_name.to_string(),
            reserved_msat: max_cost,
            max_cost_msat: max_cost,
            is_free: false,
        })
    }

    /// Finalize a charge using real usage data from the provider response.
    ///
    /// Computes actual cost, refunds the difference from the reservation.
    pub async fn finalize_charge(
        &self,
        reservation: &Reservation,
        usage: TokenUsage,
    ) -> Result<i64, BillingError> {
        if reservation.is_free {
            return Ok(0);
        }

        let pricing = self.pricing_resolver.resolve(&reservation.model).await;

        let actual_msat = usage.cost_msat(&pricing);

        let refund = reservation.reserved_msat - actual_msat;

        let request_id = reservation.request_id.clone();

        if refund < 0 {
            let available = self
                .balance_service
                .get_balance(reservation.user_id)
                .await
                .map_err(BillingError::Balance)?;
            let overage = (-refund).min(available);
            if overage > 0 {
                self.balance_service
                    .debit(reservation.user_id, overage, "overage", &request_id)
                    .await
                    .map_err(BillingError::Balance)?;
            }
        }

        if refund > 0 {
            // Credit back unused reservation
            self.balance_service
                .credit(reservation.user_id, refund, "refund", &request_id)
                .await
                .map_err(BillingError::Balance)?;
        }

        tracing::info!(
            user_id = reservation.user_id,
            model = %reservation.model,
            reserved_msat = reservation.reserved_msat,
            actual_msat,
            refund_msat = refund,
            prompt_tokens = usage.prompt_tokens,
            completion_tokens = usage.completion_tokens,
            cached_tokens = usage.cached_tokens,
            cache_write_tokens = usage.cache_write_tokens,
            "Charge finalized"
        );

        Ok(actual_msat)
    }
}

impl BillingEngine {
    pub async fn release(&self, reservation: &Reservation) -> Result<(), BillingError> {
        if reservation.is_free || reservation.reserved_msat <= 0 {
            return Ok(());
        }
        self.balance_service
            .credit(reservation.user_id, reservation.reserved_msat, "refund", &reservation.request_id)
            .await
            .map_err(BillingError::Balance)?;
        Ok(())
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
    async fn test_calculate_max_cost_with_defaults() {
        let engine = setup().await;
        let cost = engine.calculate_max_cost("gpt-4", None).await.unwrap();

        // (8192 * 5)/1_000_000 = 0 sats (truncated integer)
        // (4096 * 15)/1_000_000 = 0 sats (truncated integer)
        // + 1 sat request fee = 1 sat = 1000 msats
        assert_eq!(cost, 1000);
    }

    #[tokio::test]
    async fn test_calculate_max_cost_with_request_tokens() {
        let engine = setup().await;
        let cost = engine.calculate_max_cost("gpt-4", Some(100)).await.unwrap();
        // (100 * 5)/1_000_000 = 0 (truncated) + (4096 * 15)/1_000_000 = 0 + 1 fee = 1000 msats
        assert_eq!(cost, 1000);
    }

    #[tokio::test]
    async fn test_reserve_insufficient_funds() {
        let engine = setup().await;
        // User has 0 balance, reserve should fail
        let result = engine.reserve(1, "gpt-4", None).await;
        assert!(matches!(result, Err(BillingError::InsufficientFunds { .. })));
    }

    #[tokio::test]
    async fn test_reserve_and_finalize() {
        let engine = setup().await;

        // Credit the user enough to cover max cost
        engine
            .balance_service
            .credit(1, 5000, "deposit", "test-deposit")
            .await
            .unwrap();

        let reservation = engine.reserve(1, "gpt-4", None).await.unwrap();
        assert_eq!(reservation.reserved_msat, 1000);

        // Finalize with small usage — should get a refund
        let actual = engine
            .finalize_charge(&reservation, TokenUsage::plain(50, 100))
            .await
            .unwrap();

        // (50 * 5)/1_000_000 = 0 + (100 * 15)/1_000_000 = 0 + 1 fee = 1000 msats
        assert_eq!(actual, 1000);
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
        let sats = 200 * 1 + 600 * 100_000 / 1_000_000 + 200 * 1_250_000 / 1_000_000 + 100 * 4;
        assert_eq!(usage.cost_msat(&p), sats * 1000);
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

    #[tokio::test]
    async fn a_zero_sat_reservation_is_still_charged_for_real_usage() {
        let engine = setup().await;
        engine.balance_service.credit(1, 10_000_000, "deposit", "t").await.unwrap();
        let reservation = engine.reserve(1, "gpt-4", Some(1)).await.unwrap();
        let mut zero = reservation.clone();
        zero.reserved_msat = 0;
        zero.max_cost_msat = 0;
        assert!(!zero.is_free);
        let charged = engine.finalize_charge(&zero, TokenUsage::plain(0, 2_000_000)).await.unwrap();
        assert_eq!(charged, 31_000);
    }

    #[tokio::test]
    async fn release_returns_the_whole_reservation() {
        let engine = setup().await;
        engine.balance_service.credit(1, 5000, "deposit", "t").await.unwrap();
        let reservation = engine.reserve(1, "gpt-4", None).await.unwrap();
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 4000);
        engine.release(&reservation).await.unwrap();
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 5000);
    }

    #[tokio::test]
    async fn usage_above_the_reservation_is_charged_up_to_the_balance() {
        let engine = setup().await;
        engine.balance_service.credit(1, 5000, "deposit", "t").await.unwrap();
        let reservation = engine.reserve(1, "gpt-4", Some(10)).await.unwrap();
        assert_eq!(reservation.reserved_msat, 1000);
        let actual = engine.finalize_charge(&reservation, TokenUsage::plain(2_000_000_000, 0)).await.unwrap();
        assert_eq!(actual, 10_001_000);
        assert_eq!(engine.balance_service.get_balance(1).await.unwrap(), 0);
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

        let cost = engine.calculate_max_cost("free-model", None).await.unwrap();
        assert_eq!(cost, 0);

        let reservation = engine.reserve(1, "free-model", None).await.unwrap();
        assert!(reservation.is_free);
        assert_eq!(reservation.reserved_msat, 0);

        let actual = engine.finalize_charge(&reservation, TokenUsage::plain(1000, 1000)).await.unwrap();
        assert_eq!(actual, 0);
    }

    #[tokio::test]
    async fn test_reserve_credits_balance_correctly() {
        let engine = setup().await;

        engine
            .balance_service
            .credit(1, 5000, "deposit", "test")
            .await
            .unwrap();

        let reservation = engine.reserve(1, "gpt-4", None).await.unwrap();
        assert_eq!(reservation.reserved_msat, 1000);

        // Balance should now be 4000
        let balance = engine.balance_service.get_balance(1).await.unwrap();
        assert_eq!(balance, 4000);

        // Finalize with zero usage — full refund
        engine.finalize_charge(&reservation, TokenUsage::plain(0, 0)).await.unwrap();

        let balance = engine.balance_service.get_balance(1).await.unwrap();
        // (0 + 0 + 1sats)*1000 = 1000 msats charged, so refund of 0 (1000 reserved - 1000 actual)
        assert_eq!(balance, 4000); // 4000 was left after reserve, no refund since actual=1000=reserved
    }
}
