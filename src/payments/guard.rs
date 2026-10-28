//! Balance-checking wrapper for chat completion handlers.
//!
//! Provides `BillingGuard` that handles the admit→execute→finalize lifecycle.
//! When payments are disabled, the guard is a no-op (pass-through).

use std::sync::Arc;
use axum::http::StatusCode;
use crate::state::AppState;
use crate::payments::biller::{BillingEngine, Admission, BillingError, TokenUsage};

/// Wraps the billing lifecycle for a single chat completion request.
///
/// # Usage
///
/// ```ignore
/// let guard = BillingGuard::try_create(&state, user_id, &model).await?;
/// let response = router.chat_completions(&request).await?;
/// guard.finalize(response.usage.as_ref()).await;
/// ```
pub struct BillingGuard {
    engine: Option<Arc<BillingEngine>>,
    admission: Option<Admission>,
    settled: std::sync::atomic::AtomicBool,
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
    ) -> Result<Self, BillingError> {
        let payments = match &state.payments_state {
            Some(ps) => ps,
            None => return Ok(Self::noop()),
        };

        let Some(uid) = user_id else {
            return Ok(Self::noop());
        };

        if state.db.is_billing_exempt(uid).await.unwrap_or(false) {
            return Ok(Self::noop());
        }

        let engine = Arc::new(BillingEngine::new(
            payments.balance_service.clone(),
            payments.pricing_resolver.clone(),
        ));

        match engine.admit(uid, model).await {
            Ok(admission) => Ok(Self {
                engine: Some(engine),
                admission: Some(admission),
                settled: Default::default(),
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
            admission: None,
            settled: Default::default(),
        }
    }

    /// Finalize the charge using real usage data.
    /// Safe to call even on no-op guards.
    pub async fn finalize(&self, prompt_tokens: u32, completion_tokens: u32) -> i64 {
        self.finalize_usage(TokenUsage::plain(prompt_tokens, completion_tokens)).await
    }

    pub async fn finalize_usage(&self, usage: TokenUsage) -> i64 {
        let (Some(engine), Some(admission)) = (&self.engine, &self.admission) else {
            return 0;
        };
        if self.settled.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return 0;
        }
        match engine.finalize_charge(admission, usage).await {
            Ok(billed_msat) => billed_msat,
            Err(e) => {
                tracing::error!(error = %e, "Failed to finalize billing charge");
                0
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
        let db = Arc::new(Database::test().await);
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
            discovery_config: None,
            admin_ui_path: "/app/admin/dist".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        };

        Arc::new(AppState {
            config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(crate::auth::admin::SessionStore::new()),
            db,
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
        })
    }

    #[tokio::test]
    async fn test_noop_guard_when_payments_disabled() {
        let state = setup().await;
        let guard = BillingGuard::try_create(&state, Some(1), "gpt-4")
            .await
            .unwrap();
        assert!(!guard.is_active());
        assert_eq!(guard.finalize(100, 200).await, 0);
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
