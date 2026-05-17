//! Billing engine — max cost calculation, balance reservation, charge finalization.
//!
//! Follows RIP-05 pricing: max_cost is computed upfront from context window and
//! max output tokens. The actual charge uses real usage data from the provider response.

use std::sync::Arc;
use crate::payments::balance::{BalanceService, BalanceError};
use crate::payments::pricing::PricingResolver;

/// Handles the billing lifecycle for a single request:
/// resolve pricing → compute max_cost → reserve → finalize with real usage.
pub struct BillingEngine {
    balance_service: Arc<BalanceService>,
    pricing_resolver: Arc<PricingResolver>,
}

/// Result of reserving balance for a request.
#[derive(Debug, Clone)]
pub struct Reservation {
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

        if max_cost == 0 || pricing.is_free {
            return Ok(Reservation {
                user_id,
                model: model_name.to_string(),
                reserved_msat: 0,
                max_cost_msat: 0,
                is_free: true,
            });
        }

        let request_id = format!("req-{}", chrono::Utc::now().timestamp_millis());

        self.balance_service
            .debit(user_id, max_cost, "reserve", &request_id)
            .await
            .map_err(|e| match e {
                BalanceError::InsufficientFunds { required, available } => {
                    BillingError::InsufficientFunds { required, available }
                }
                other => BillingError::Balance(other),
            })?;

        Ok(Reservation {
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
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> Result<i64, BillingError> {
        if reservation.is_free {
            return Ok(0);
        }

        let pricing = self.pricing_resolver.resolve(&reservation.model).await;

        // Actual cost in sats
        let input_cost = (prompt_tokens as i64 * pricing.price_per_1m_input_sats) / 1_000_000;
        let output_cost = (completion_tokens as i64 * pricing.price_per_1m_output_sats) / 1_000_000;
        let request_cost = pricing.price_per_request_sats;

        let actual_sats = input_cost + output_cost + request_cost;
        let actual_msat = actual_sats * 1000;

        let refund = reservation.reserved_msat - actual_msat;

        let request_id = format!("charge-{}", chrono::Utc::now().timestamp_millis());

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
            prompt_tokens,
            completion_tokens,
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
            .finalize_charge(&reservation, 50, 100)
            .await
            .unwrap();

        // (50 * 5)/1_000_000 = 0 + (100 * 15)/1_000_000 = 0 + 1 fee = 1000 msats
        assert_eq!(actual, 1000);
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

        let actual = engine.finalize_charge(&reservation, 1000, 1000).await.unwrap();
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
        engine.finalize_charge(&reservation, 0, 0).await.unwrap();

        let balance = engine.balance_service.get_balance(1).await.unwrap();
        // (0 + 0 + 1sats)*1000 = 1000 msats charged, so refund of 0 (1000 reserved - 1000 actual)
        assert_eq!(balance, 4000); // 4000 was left after reserve, no refund since actual=1000=reserved
    }
}
