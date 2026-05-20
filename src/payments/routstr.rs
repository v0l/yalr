//! Routstr-compatible API endpoints.
//!
//! These endpoints follow the Routstr protocol (RIP-01, RIP-08).
//! All are feature-gated behind `payments.enabled` in config.yaml.
//!
//! Cashu is intentionally not supported — Lightning (Bolt11 via LND) is the
//! sole payment method. Balances are tracked internally in SQLite.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::auth::admin::AuthenticatedUser;
use crate::state::AppState;

/// GET /v1/info — Node information (per RIP-01)
pub async fn routstr_info(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let payments = match &state.payments_state {
        Some(ps) => ps,
        None => return payments_disabled(),
    };

    Json(serde_json::json!({
        "name": "yalr",
        "description": "YALR - LLM Router",
        "version": env!("CARGO_PKG_VERSION"),
        "payments": {
            "enabled": true,
            "methods": if payments.lightning_service.is_some() { vec!["lightning"] } else { vec![] },
        },
    }))
    .into_response()
}

/// GET /v1/balance/info — Current balance for authenticated user (per RIP-01)
pub async fn balance_info(
    State(state): State<Arc<AppState>>,
    Extension(authenticated_user): Extension<AuthenticatedUser>,
) -> axum::response::Response {
    let user = &authenticated_user.user;
    let payments = match &state.payments_state {
        Some(ps) => ps,
        None => return payments_disabled(),
    };

    match payments.balance_service.get_balance(user.id).await {
        Ok(balance_msat) => {
            let sats = balance_msat / 1000;
            Json(serde_json::json!({
                "balance_msat": balance_msat,
                "balance_sats": sats,
                "available_sats": sats,
            }))
            .into_response()
        }
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to get balance");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to get balance"})),
            )
                .into_response()
        }
    }
}

/// POST /v1/balance/refund — Refund remaining balance via Lightning
///
/// The entire remaining balance is sent back to the user as a Lightning payment.
/// Body must contain an `invoice` (Bolt11) to pay to.
pub async fn balance_refund(
    State(state): State<Arc<AppState>>,
    Extension(authenticated_user): Extension<AuthenticatedUser>,
    Json(body): Json<RefundRequest>,
) -> axum::response::Response {
    let user = &authenticated_user.user;
    let payments = match &state.payments_state {
        Some(ps) => ps,
        None => return payments_disabled(),
    };

    let lightning = match &payments.lightning_service {
        Some(ls) => ls.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Lightning payments not configured on this node"})),
            )
                .into_response()
        }
    };

    // Check balance
    let balance = match payments.balance_service.get_balance(user.id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to get balance for refund");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to get balance"}))).into_response();
        }
    };

    if balance <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "No balance to refund"})),
        )
            .into_response();
    }

    // Debit the full balance first (atomic)
    let ref_id = format!("refund-{}", chrono::Utc::now().timestamp_millis());
    match payments.balance_service.debit(user.id, balance, "refund", &ref_id).await {
        Ok(_) => {}
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to debit balance for refund");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "Failed to process refund"}))).into_response();
        }
    }

    // Pay the user's invoice
    match lightning.pay_invoice(&body.invoice, body.amount_sats.map(|a| a)).await {
        Ok(result) => {
            tracing::info!(
                user_id = user.id,
                refunded_msat = balance,
                payment_hash = %result.payment_hash,
                "Balance refunded via Lightning"
            );
            Json(serde_json::json!({
                "refunded_msat": balance,
                "refunded_sats": balance / 1000,
                "payment_hash": result.payment_hash,
            }))
            .into_response()
        }
        Err(e) => {
            // Credit back the balance since the payment failed
            tracing::error!(user_id = user.id, error = %e, "Refund payment failed, crediting back balance");
            let _ = payments.balance_service.credit(user.id, balance, "refund_reversal", &ref_id).await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Refund payment failed: {}", e)})),
            )
                .into_response()
        }
    }
}

/// POST /lightning/invoice — Create a Bolt11 invoice for top-up (per RIP-08)
pub async fn create_lightning_invoice(
    State(state): State<Arc<AppState>>,
    Extension(authenticated_user): Extension<AuthenticatedUser>,
    Json(body): Json<CreateInvoiceRequest>,
) -> axum::response::Response {
    let user = &authenticated_user.user;
    let payments = match &state.payments_state {
        Some(ps) => ps,
        None => return payments_disabled(),
    };

    let lightning = match &payments.lightning_service {
        Some(ls) => ls.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Lightning payments not configured on this node"})),
            )
                .into_response()
        }
    };

    match lightning
        .create_invoice(user.id, body.amount_sats, &body.memo, body.expire_seconds)
        .await
    {
        Ok(instruction) => {
            // Return the instruction wrapped in a TopupResponse
            use crate::payments::instructions::{TopupResponse, ProviderInfo};
            (
                StatusCode::CREATED,
                Json(TopupResponse {
                    provider: ProviderInfo {
                        slug: "user-balance".to_string(),
                        name: "User Balance".to_string(),
                    },
                    instruction,
                    message: Some("Pay this invoice to top up your balance".to_string()),
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to create Lightning invoice");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to create invoice: {}", e)})),
            )
                .into_response()
        }
    }
}

/// GET /lightning/invoice/:payment_hash/status — Check invoice payment status (per RIP-08)
pub async fn check_lightning_invoice(
    State(state): State<Arc<AppState>>,
    Path(payment_hash): Path<String>,
) -> axum::response::Response {
    let payments = match &state.payments_state {
        Some(ps) => ps,
        None => return payments_disabled(),
    };

    let lightning = match &payments.lightning_service {
        Some(ls) => ls.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Lightning payments not configured on this node"})),
            )
                .into_response()
        }
    };

    match lightning.check_invoice(&payment_hash).await {
        Ok(invoice) => {
            Json(serde_json::json!({
                "status": invoice.status,
                "payment_hash": invoice.payment_hash,
                "amount_sats": invoice.amount_sats,
                "created_at": invoice.created_at,
                "expires_at": invoice.expires_at,
            }))
            .into_response()
        }
        Err(crate::payments::lightning::LightningError::NotFound(_)) => {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "Invoice not found"})))
                .into_response()
        }
        Err(e) => {
            tracing::error!(payment_hash = %payment_hash, error = %e, "Failed to check invoice");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// POST /providers/:slug/lightning/invoice — Create invoice for provider top-up
///
/// Calls the upstream Routstr provider's /lightning/invoice endpoint to generate
/// an invoice for topping up the provider's balance.
pub async fn create_provider_invoice(
    State(state): State<Arc<AppState>>,
    Extension(_authenticated_user): Extension<AuthenticatedUser>, // Admin authentication
    Path(slug): Path<String>,
    Json(body): Json<CreateInvoiceRequest>,
) -> axum::response::Response {
    use reqwest::Client;

    // Find the provider
    let provider = match state.db.get_provider_by_slug(&slug).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Provider not found"})),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "Failed to get provider");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Failed to get provider"})),
            )
                .into_response()
        }
    };

    // Check if it's a Routstr provider
    if provider.provider_type != crate::db::ProviderType::Routstr {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provider is not a Routstr provider"})),
        )
            .into_response()
    }

    // Build the upstream invoice request - routstr expects /v1/lightning/invoice
    let upstream_url = match url::Url::parse(&provider.base_url) {
        Ok(u) => u.join("v1/lightning/invoice").unwrap_or_else(|_| url::Url::parse("/v1/lightning/invoice").unwrap()),
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "Invalid provider base URL");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Invalid provider base URL"})),
            )
                .into_response()
        }
    };

    tracing::info!(slug = %slug, url = %upstream_url.as_str(), "Creating provider invoice via upstream");

    // Get a valid model name for the provider (optional for top-ups)
    // Try to get models from routing_config_providers (which stores the actual model name)
    let model_name = {
        // Query routing config providers for this provider to get associated model names
        let routing_config_providers = state.db
            .list_routing_config_providers_for_provider(provider.id)
            .await
            .unwrap_or_default();
        
        // Find the first active entry with a non-empty model name
        routing_config_providers
            .iter()
            .find(|rcp| rcp.is_active && rcp.model.as_ref().map(|m| !m.is_empty()).unwrap_or(false))
            .and_then(|rcp| rcp.model.clone())
    };

    // Build the upstream request body - routstr expects:
    // - amount_sats: required
    // - purpose: "create" or "topup" (required)
    // - model: optional (only needed if routing to a specific model)
    // For general top-ups, we can omit the model or send empty string
    let mut upstream_body = serde_json::json!({
        "amount_sats": body.amount_sats,
        "purpose": "topup",
    });
    
    // Add model if we have a valid one
    if let Some(name) = model_name {
        upstream_body["model"] = serde_json::json!(name);
    }

    let mut request_builder = Client::new()
        .post(upstream_url.as_str())
        .json(&upstream_body);

    // Add API key if present (routstr requires auth for topup)
    if let Some(ref api_key) = provider.api_key {
        request_builder = request_builder.bearer_auth(api_key);
    }

    // Call the upstream endpoint
    match request_builder.send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                let error_text = resp.text().await.unwrap_or_default();
                tracing::error!(slug = %slug, status = %status, error = %error_text, "Upstream invoice creation failed");
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": format!("Upstream error: {}", error_text)})),
                )
                    .into_response()
            }

            let upstream_invoice: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(slug = %slug, error = %e, "Failed to parse upstream response");
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "Failed to parse upstream response"})),
                    )
                        .into_response()
                }
            };

            // Parse the upstream response and convert to PaymentInstruction
            use crate::payments::instructions::{PaymentInstruction, TopupResponse, ProviderInfo};
            
            let bolt11 = upstream_invoice.get("bolt11").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let payment_hash = upstream_invoice.get("payment_hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let expires_at = upstream_invoice.get("expires_at").and_then(|v| v.as_str());
            
            let instruction = if bolt11.is_empty() {
                // If no bolt11, try to extract a redirect URL
                if let Some(url) = upstream_invoice.get("payment_url").and_then(|v| v.as_str()) {
                    PaymentInstruction::Redirect {
                        url: url.to_string(),
                        amount_usd: None,
                        session_token: None,
                    }
                } else {
                    PaymentInstruction::Manual {
                        instructions: format!("Contact provider {} for payment instructions", slug),
                        amount_usd: None,
                        reference_code: None,
                    }
                }
            } else {
                PaymentInstruction::LightningBolt11 {
                    bolt11,
                    payment_hash,
                    amount_sats: body.amount_sats as i64,
                    amount_msat: (body.amount_sats * 1000) as i64,
                    memo: Some(format!("Top-up for {}", slug)),
                    expires_at: expires_at.and_then(|s| {
                        chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                            .ok()
                            .map(|dt| dt.and_utc().timestamp())
                    }),
                    invoice_id: None,
                }
            };

            // Return wrapped in TopupResponse
            (
                StatusCode::CREATED,
                Json(TopupResponse {
                    provider: ProviderInfo {
                        slug: slug.clone(),
                        name: provider.name.clone(),
                    },
                    instruction,
                    message: Some(format!("Complete payment to top up {} balance", slug)),
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "Failed to connect to upstream provider");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("Failed to connect to upstream provider: {}", e)})),
            )
                .into_response()
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn payments_disabled() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "Payments not enabled"})),
    )
        .into_response()
}

// ── Request types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RefundRequest {
    /// Bolt11 invoice to pay the refund to
    pub invoice: String,
    /// Optional: max amount in sats to refund (defaults to full balance)
    pub amount_sats: Option<u64>,
}

#[derive(Deserialize)]
pub struct CreateInvoiceRequest {
    pub amount_sats: u64,
    #[serde(default = "default_invoice_memo")]
    pub memo: String,
    pub expire_seconds: Option<u32>,
}

fn default_invoice_memo() -> String {
    "YALR balance top-up".to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invoice_request_deserialization() {
        let json = r#"{"amount_sats": 1000, "memo": "test memo", "expire_seconds": 1800}"#;
        let req: CreateInvoiceRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.amount_sats, 1000);
        assert_eq!(req.memo, "test memo");
        assert_eq!(req.expire_seconds, Some(1800));
    }

    #[test]
    fn test_invoice_request_default_memo() {
        let json = r#"{"amount_sats": 500}"#;
        let req: CreateInvoiceRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.amount_sats, 500);
        assert_eq!(req.memo, "YALR balance top-up");
        assert_eq!(req.expire_seconds, None);
    }

    #[test]
    fn test_refund_request_deserialization() {
        let json = r#"{"invoice": "lnbc...", "amount_sats": 1000}"#;
        let req: RefundRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.invoice, "lnbc...");
        assert_eq!(req.amount_sats, Some(1000));
    }
}
