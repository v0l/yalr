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
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::auth::admin::AuthenticatedUser;
use crate::state::AppState;

// ── Helpers ──────────────────────────────────────────────────────────────

/// Simple error response helper — replaces repetitive `serde_json::json!({ "error": ... })`
fn error_response(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(ErrorResponse { error: message.into() })).into_response()
}

fn payments_disabled() -> axum::response::Response {
    error_response(StatusCode::NOT_FOUND, "Payments not enabled")
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

// ── Response types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Serialize)]
struct RoutstrInfoResponse {
    name: String,
    description: String,
    version: &'static str,
    npub: Option<String>,
    http_url: Option<String>,
    onion_url: Option<String>,
    mints: Vec<String>,
    payments: RoutstrInfoPayments,
}

#[derive(Serialize)]
struct RoutstrInfoPayments {
    enabled: bool,
    methods: Vec<&'static str>,
}

#[derive(Serialize)]
struct BalanceInfoResponse {
    balance: i64,
    reserved: i64,
    balance_msat: i64,
    balance_sats: i64,
    available_sats: i64,
}

#[derive(Serialize)]
struct RefundResponse {
    refunded_msat: i64,
    refunded_sats: i64,
    payment_hash: String,
}

#[derive(Serialize)]
struct InvoiceStatusResponse {
    status: String,
    payment_hash: String,
    amount_sats: i64,
    created_at: crate::db::Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<crate::db::Timestamp>,
}

/// Upstream invoice request body (RIP-08)
#[derive(Serialize)]
struct ProviderInvoiceRequest {
    amount_sats: u64,
    purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

/// Upstream invoice response from a Routstr provider
#[derive(Deserialize)]
struct UpstreamInvoiceResponse {
    #[serde(default)]
    bolt11: Option<String>,
    #[serde(default)]
    payment_hash: Option<String>,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    payment_url: Option<String>,
}

// ── Endpoints ─────────────────────────────────────────────────────────────

/// GET /v1/info — Node information (per RIP-01)
pub async fn routstr_info(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let discovery = state.config.discovery_config.as_ref().filter(|d| d.enabled);
    let npub = match discovery {
        Some(d) => crate::discovery::npub(d, &state.db).await,
        None => None,
    };
    let lightning = state
        .payments_state
        .as_ref()
        .is_some_and(|p| p.lightning_service.is_some());

    Json(RoutstrInfoResponse {
        name: discovery.and_then(|d| d.name.clone()).unwrap_or_else(|| "yalr".to_string()),
        description: discovery
            .and_then(|d| d.about.clone())
            .unwrap_or_else(|| "YALR - LLM Router".to_string()),
        version: env!("CARGO_PKG_VERSION"),
        npub,
        http_url: discovery.map(|d| d.http_url.trim_end_matches('/').to_string()),
        onion_url: discovery.and_then(|d| d.onion_url.clone()),
        mints: Vec::new(),
        payments: RoutstrInfoPayments {
            enabled: lightning,
            methods: if lightning { vec!["lightning"] } else { vec![] },
        },
    })
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
            Json(BalanceInfoResponse {
                balance: balance_msat,
                reserved: 0,
                balance_msat,
                balance_sats: sats,
                available_sats: sats,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to get balance");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get balance")
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
        None => return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node"),
    };

    // Check balance
    let balance = match payments.balance_service.get_balance(user.id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Failed to get balance for refund");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get balance");
        }
    };

    if balance <= 0 {
        return error_response(StatusCode::BAD_REQUEST, "No balance to refund");
    }

    let amount_msat = match invoice_amount_msat(&body.invoice) {
        Ok(a) => a,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    if amount_msat > balance {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invoice amount {} msat exceeds balance {} msat", amount_msat, balance),
        );
    }

    let ref_id = format!("refund-{}", chrono::Utc::now().timestamp_millis());
    if let Err(e) = payments.balance_service.debit(user.id, amount_msat, "refund", &ref_id).await {
        tracing::error!(user_id = user.id, error = %e, "Failed to debit balance for refund");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to process refund");
    }

    match lightning.pay_invoice(&body.invoice, None).await {
        Ok(result) => {
            let remaining = balance - amount_msat;
            let fee = (result.fee_msat as i64).min(remaining);
            if fee > 0 {
                let _ = payments
                    .balance_service
                    .debit(user.id, fee, "refund_fee", &ref_id)
                    .await;
            }
            tracing::info!(
                user_id = user.id,
                refunded_msat = amount_msat,
                fee_msat = result.fee_msat,
                payment_hash = %result.payment_hash,
                "Balance refunded via Lightning"
            );
            Json(RefundResponse {
                refunded_msat: amount_msat,
                refunded_sats: amount_msat / 1000,
                payment_hash: result.payment_hash,
            })
            .into_response()
        }
        Err(e) => {
            tracing::error!(user_id = user.id, error = %e, "Refund payment failed, crediting back balance");
            let _ = payments
                .balance_service
                .credit(user.id, amount_msat, "refund_reversal", &ref_id)
                .await;
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Refund payment failed: {}", e))
        }
    }
}

fn invoice_amount_msat(invoice: &str) -> Result<i64, &'static str> {
    let parsed = payments_rs::lightning::AddInvoiceResponse::from_invoice(invoice.trim(), None)
        .map_err(|_| "Invalid bolt11 invoice")?;
    let msat = parsed
        .parsed_invoice
        .amount_milli_satoshis()
        .ok_or("Invoice must specify an amount")?;
    if msat == 0 {
        return Err("Invoice must specify an amount");
    }
    i64::try_from(msat).map_err(|_| "Invoice amount too large")
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
        None => return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node"),
    };

    match lightning
        .create_invoice(user.id, body.amount_sats, &body.memo, body.expire_seconds)
        .await
    {
        Ok(instruction) => {
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
            error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create invoice: {}", e))
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
        None => return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node"),
    };

    match lightning.check_invoice(&payment_hash).await {
        Ok(invoice) => {
            Json(InvoiceStatusResponse {
                status: invoice.status,
                payment_hash: invoice.payment_hash,
                amount_sats: invoice.amount_sats,
                created_at: invoice.created_at,
                expires_at: invoice.expires_at,
            })
            .into_response()
        }
        Err(crate::payments::lightning::LightningError::NotFound(_)) => {
            error_response(StatusCode::NOT_FOUND, "Invoice not found")
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
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Provider not found"),
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "Failed to get provider");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to get provider");
        }
    };

    // Check if it's a Routstr provider
    if provider.provider_type != crate::db::ProviderType::Routstr {
        return error_response(StatusCode::BAD_REQUEST, "Provider is not a Routstr provider");
    }

    // Build the upstream invoice request - routstr expects /v1/lightning/invoice
    let upstream_url = match url::Url::parse(&provider.base_url) {
        Ok(u) => u.join("v1/lightning/invoice").unwrap_or_else(|_| url::Url::parse("/v1/lightning/invoice").unwrap()),
        Err(e) => {
            tracing::error!(slug = %slug, error = %e, "Invalid provider base URL");
            return error_response(StatusCode::BAD_REQUEST, "Invalid provider base URL");
        }
    };

    tracing::info!(slug = %slug, url = %upstream_url.as_str(), "Creating provider invoice via upstream");

    // Get a valid model name for the provider (optional for top-ups)
    let model_name = {
        let routing_config_providers = state.db
            .list_routing_config_providers_for_provider(provider.id)
            .await
            .unwrap_or_default();
        
        routing_config_providers
            .iter()
            .find(|rcp| rcp.is_active && rcp.model.as_ref().map(|m| !m.is_empty()).unwrap_or(false))
            .and_then(|rcp| rcp.model.clone())
    };

    let upstream_body = ProviderInvoiceRequest {
        amount_sats: body.amount_sats,
        purpose: "topup".to_string(),
        model: model_name,
    };

    let mut request_builder = Client::new()
        .post(upstream_url.as_str())
        .json(&upstream_body);

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
                return error_response(StatusCode::BAD_REQUEST, format!("Upstream error: {}", error_text));
            }

            let upstream_invoice: UpstreamInvoiceResponse = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(slug = %slug, error = %e, "Failed to parse upstream response");
                    return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to parse upstream response");
                }
            };

            // Parse the upstream response and convert to PaymentInstruction
            use crate::payments::instructions::{PaymentInstruction, TopupResponse, ProviderInfo};
            
            let instruction = match upstream_invoice.bolt11 {
                Some(bolt11) if !bolt11.is_empty() => PaymentInstruction::LightningBolt11 {
                    bolt11,
                    payment_hash: upstream_invoice.payment_hash.unwrap_or_default(),
                    amount_sats: body.amount_sats as i64,
                    amount_msat: (body.amount_sats * 1000) as i64,
                    memo: Some(format!("Top-up for {}", slug)),
                    expires_at: upstream_invoice.expires_at,
                    invoice_id: None,
                },
                _ => match upstream_invoice.payment_url {
                    Some(url) => PaymentInstruction::Redirect {
                        url,
                        amount_usd: None,
                        session_token: None,
                    },
                    None => PaymentInstruction::Manual {
                        instructions: format!("Contact provider {} for payment instructions", slug),
                        amount_usd: None,
                        reference_code: None,
                    },
                },
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
            error_response(StatusCode::BAD_GATEWAY, format!("Failed to connect to upstream provider: {}", e))
        }
    }
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

#[cfg(test)]
mod refund_tests {
    use super::*;

    use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
    use secp256k1::{hashes::{sha256, Hash}, Secp256k1, SecretKey};

    fn invoice(amount_msat: Option<u64>) -> String {
        let key = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let hash = sha256::Hash::from_slice(&[1u8; 32]).unwrap();
        let builder = InvoiceBuilder::new(Currency::Bitcoin)
            .description("test".into())
            .payment_hash(hash)
            .payment_secret(PaymentSecret([2u8; 32]))
            .duration_since_epoch(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap())
            .min_final_cltv_expiry_delta(144);
        let builder = match amount_msat {
            Some(a) => builder.amount_milli_satoshis(a),
            None => builder,
        };
        builder
            .build_signed(|m| Secp256k1::new().sign_ecdsa_recoverable(m, &key))
            .unwrap()
            .to_string()
    }

    #[test]
    fn reads_the_invoice_amount() {
        assert_eq!(invoice_amount_msat(&invoice(Some(250_000_000))), Ok(250_000_000));
    }

    #[test]
    fn refuses_amountless_and_garbage_invoices() {
        assert!(invoice_amount_msat(&invoice(None)).is_err());
        assert!(invoice_amount_msat("not an invoice").is_err());
    }
}
