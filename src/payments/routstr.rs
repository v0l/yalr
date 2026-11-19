//! Routstr-compatible API endpoints.
//!
//! These endpoints follow the Routstr protocol (RIP-01, RIP-08).
//! All are feature-gated behind `payments.enabled` in config.yaml.
//!
//! Cashu is intentionally not supported. Lightning invoices come from the
//! payment gateway. Balances are tracked in the database.

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
        .is_some_and(|p| p.gateway.as_ref().is_some_and(|g| g.currency_allowed(crate::db::SATS)));

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

    let Some(gateway) = payments.gateway.clone() else {
        return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node");
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

    match refund_against_deposits(&state, &gateway, user.id, amount_msat, &body.invoice).await {
        Ok(payment_hash) => {
            tracing::info!(user_id = user.id, refunded_msat = amount_msat, "Balance refunded via the gateway");
            Json(RefundResponse { refunded_msat: amount_msat, refunded_sats: amount_msat / 1000, payment_hash }).into_response()
        }
        Err(message) => {
            tracing::error!(user_id = user.id, error = %message, "Refund failed, crediting back balance");
            let _ = payments.balance_service.credit(user.id, amount_msat, "refund_reversal", &ref_id).await;
            error_response(StatusCode::BAD_REQUEST, format!("Refund payment failed: {message}"))
        }
    }
}

async fn refund_against_deposits(
    state: &AppState,
    gateway: &crate::payments::gateway::GatewayService,
    user_id: i64,
    amount_msat: i64,
    invoice: &str,
) -> Result<String, String> {
    let deposits = state
        .db
        .recent_completed_gateway_payments(user_id, crate::db::SATS, 20)
        .await
        .map_err(|e| e.to_string())?;
    let mut last = "no Lightning deposit is large enough to refund from".to_string();
    for deposit in deposits.into_iter().filter(|d| d.amount_milli >= amount_msat) {
        let Some(remote) = deposit.gateway_payment_id.as_deref() else { continue };
        match gateway.client.refund_payment(remote, &payment_gateway_client::RefundInput { amount: Some(amount_msat as u64), destination: Some(invoice.to_string()), attempt_id: None }).await {
            Ok(_) => return Ok(deposit.payment_hash.unwrap_or_default()),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn invoice_amount_msat(invoice: &str) -> Result<i64, &'static str> {
    let parsed = invoice.trim().parse::<lightning_invoice::Bolt11Invoice>().map_err(|_| "Invalid bolt11 invoice")?;
    let msat = parsed
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

    let Some(gateway) = payments.gateway.clone() else {
        return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node");
    };
    let amount_milli = match gateway.check_amount(crate::db::SATS, body.amount_sats as f64) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e.to_string()),
    };

    match gateway.topup(user.id, crate::db::SATS, amount_milli, false, None).await {
        Ok(payment) => {
            use crate::payments::instructions::{PaymentInstruction, ProviderInfo, TopupResponse};
            let (Some(bolt11), Some(payment_hash)) = (payment.bolt11.clone(), payment.payment_hash.clone()) else {
                return error_response(StatusCode::BAD_GATEWAY, "The gateway did not return a Lightning invoice");
            };
            (
                StatusCode::CREATED,
                Json(TopupResponse {
                    provider: ProviderInfo { slug: "user-balance".to_string(), name: "User Balance".to_string() },
                    instruction: PaymentInstruction::LightningBolt11 {
                        bolt11,
                        payment_hash,
                        amount_sats: body.amount_sats as i64,
                        amount_msat: amount_milli,
                        memo: Some(body.memo.clone()),
                        expires_at: payment.expires_at.map(|t| t.timestamp()),
                        invoice_id: Some(payment.id),
                    },
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

    let Some(gateway) = payments.gateway.clone() else {
        return error_response(StatusCode::NOT_FOUND, "Lightning payments not configured on this node");
    };

    let found = match state.db.gateway_payment_by_hash(&payment_hash).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(payment_hash = %payment_hash, error = %e, "Failed to check invoice");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(mut payment) = found {
        if payment.status == "pending" {
            if let Some(remote) = payment.gateway_payment_id.clone() {
                let _ = gateway.reconcile(&remote).await;
                payment = state.db.gateway_payment(&payment.public_id).await.ok().flatten().unwrap_or(payment);
            }
        }
        let expired = payment.status == "pending" && payment.expires_at.is_some_and(|t| chrono::Utc::now() > t);
        let status = match payment.status.as_str() {
            "completed" => "paid".to_string(),
            "failed" => "cancelled".to_string(),
            _ if expired => "expired".to_string(),
            other => other.to_string(),
        };
        return Json(InvoiceStatusResponse {
            status,
            payment_hash,
            amount_sats: payment.amount_milli / 1000,
            created_at: payment.created_at,
            expires_at: payment.expires_at,
        })
        .into_response();
    }
    match state.db.public_invoice_by_hash(&payment_hash).await {
        Ok(Some(row)) => Json(InvoiceStatusResponse {
            status: row.status,
            payment_hash,
            amount_sats: row.amount_sats,
            created_at: row.created_at,
            expires_at: row.expires_at,
        })
        .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Invoice not found"),
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
