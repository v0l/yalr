//! Balance-checking wrapper for chat completion handlers.
//!
//! Provides `BillingGuard` that handles the reserve→execute→finalize lifecycle.
//! When payments are disabled, the guard is a no-op (pass-through).

use std::sync::Arc;
use axum::http::StatusCode;
use crate::state::AppState;
use crate::payments::biller::{BillingEngine, Reservation, BillingError};

/// Wraps the billing lifecycle for a single chat completion request.
///
/// # Usage
///
/// ```ignore
/// let guard = BillingGuard::try_reserve(&state, user_id, &model, max_tokens).await?;
/// let response = router.chat_completions(&request).await?;
/// guard.finalize(response.usage.as_ref()).await;
/// ```
pub struct BillingGuard {
    engine: Option<Arc<BillingEngine>>,
    reservation: Option<Reservation>,
}

impl BillingGuard {
    /// Create a new billing guard.
    ///
    /// Returns `None` (no billing) if payments are disabled or if the
    /// authenticated user cannot be identified.
    pub async fn try_create(
        state: &Arc<AppState>,
        user_id: Option<i64>,
        model: &str,
        max_tokens: Option<u32>,
    ) -> Result<Self, BillingError> {
        let payments = match &state.payments_state {
            Some(ps) => ps,
            None => return Ok(Self::noop()),
        };

        let Some(uid) = user_id else {
            return Ok(Self::noop());
        };

        let engine = Arc::new(BillingEngine::new(
            payments.balance_service.clone(),
            payments.pricing_resolver.clone(),
        ));

        match engine.reserve(uid, model, max_tokens).await {
            Ok(reservation) => Ok(Self {
                engine: Some(engine),
                reservation: Some(reservation),
            }),
            Err(BillingError::InsufficientFunds { required, available }) => {
                // Re-throw insufficient funds as a distinct error for HTTP 402
                Err(BillingError::InsufficientFunds { required, available })
            }
            Err(e) => {
                tracing::error!(error = %e, "Billing reservation error");
                Err(e)
            }
        }
    }

    /// Creates a no-op guard (no billing).
    fn noop() -> Self {
        Self {
            engine: None,
            reservation: None,
        }
    }

    /// Finalize the charge using real usage data.
    /// Safe to call even on no-op guards.
    pub async fn finalize(&self, prompt_tokens: u32, completion_tokens: u32) {
        if let (Some(engine), Some(reservation)) = (&self.engine, &self.reservation) {
            if let Err(e) = engine.finalize_charge(reservation, prompt_tokens, completion_tokens).await {
                tracing::error!(error = %e, "Failed to finalize billing charge");
            }
        }
    }

    /// Check if billing is active for this request.
    pub fn is_active(&self) -> bool {
        self.engine.is_some()
    }
}

/// Structured error body for 402 Payment Required responses.
#[derive(serde::Serialize)]
struct PaymentRequiredError {
    error: PaymentRequiredErrorDetail,
}

#[derive(serde::Serialize)]
struct PaymentRequiredErrorDetail {
    message: &'static str,
    #[serde(rename = "type")]
    error_type: &'static str,
    required_msat: i64,
    available_msat: i64,
}

/// Convert a BillingError::InsufficientFunds into a 402 Payment Required response.
pub fn insufficient_funds_response(required: i64, available: i64) -> (StatusCode, axum::Json<serde_json::Value>) {
    // Return as Json<Value> for API compatibility — this is a public function
    // that callers may depend on the Value return type.
    tracing::info!(
        required_msat = required,
        available_msat = available,
        "Payment required"
    );
    let body = serde_json::to_value(PaymentRequiredError {
        error: PaymentRequiredErrorDetail {
            message: "Insufficient funds",
            error_type: "payment_required",
            required_msat: required,
            available_msat: available,
        },
    }).unwrap();
    (StatusCode::PAYMENT_REQUIRED, axum::Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, DatabaseConfig, PaymentsConfig, ServerConfig};
    use crate::db::Database;
    use crate::metrics::MetricsStore;

    async fn setup() -> Arc<AppState> {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let metrics_store = Arc::new(MetricsStore::new(100));
        let router = Arc::new(crate::router::engine::Router::new(
            (*metrics_store).clone(),
            db.clone(),
        ));

        let config = AppConfig {
            db: db.clone(),
            router,
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None, // payments disabled → noop guard
            admin_ui_path: "/app/admin/dist".to_string(),
        };

        Arc::new(AppState {
            config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(crate::auth::admin::SessionStore::new()),
            db,
            payments_state: None,
        })
    }

    #[tokio::test]
    async fn test_noop_guard_when_payments_disabled() {
        let state = setup().await;
        let guard = BillingGuard::try_create(&state, Some(1), "gpt-4", None)
            .await
            .unwrap();
        assert!(!guard.is_active());
        // Finalize should not panic
        guard.finalize(100, 200).await;
    }

    #[tokio::test]
    async fn test_payment_required_response() {
        let (status, json) = insufficient_funds_response(5000, 1000);
        assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
        let body = json.0;
        assert_eq!(body["error"]["type"], "payment_required");
        assert_eq!(body["error"]["required_msat"], 5000);
        assert_eq!(body["error"]["available_msat"], 1000);
    }
}
