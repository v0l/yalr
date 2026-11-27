use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::api_key::{get_last_four, hash_api_key};
use crate::db::gateway_payments::KIND_ANONYMOUS;
use crate::db::{random_hex, Database, NewApiKey, NewUser, PublicInvoice, UserType, SATS};
use crate::payments::provider::{PaymentProvider, TopupRequest};
use crate::state::AppState;

const MIN_SATS: u64 = 100;
const MAX_SATS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Create,
    Topup,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Purpose::Create => "create",
            Purpose::Topup => "topup",
        }
    }
}

#[derive(Deserialize)]
pub struct InvoiceRequest {
    pub amount_sats: u64,
    pub purpose: Purpose,
}

#[derive(Deserialize)]
pub struct RecoverRequest {
    pub bolt11: String,
}

#[derive(Serialize)]
struct InvoiceCreated {
    invoice_id: String,
    bolt11: String,
    amount_sats: i64,
    expires_at: i64,
    payment_hash: String,
}

#[derive(Serialize)]
struct InvoiceStatus {
    status: String,
    api_key: Option<String>,
    amount_sats: i64,
    paid_at: Option<i64>,
    created_at: i64,
    expires_at: i64,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ErrorBody { error: message.into() })).into_response()
}

pub fn derive_api_key(secret: &str, public_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(public_id.as_bytes());
    format!("sk-{}", hex::encode(hasher.finalize()))
}


fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

pub fn spawn_unpaid_pruner(db: std::sync::Arc<crate::db::Database>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            interval.tick().await;
            match db.prune_unpaid_anonymous_users().await {
                Ok(0) => {}
                Ok(n) => tracing::info!(users = n, "Pruned unpaid anonymous users"),
                Err(e) => tracing::error!(error = %e, "Anonymous user prune failed"),
            }
        }
    });
}

async fn create_account(db: &Database, secret: &str, public_id: &str) -> Result<i64, sqlx::Error> {
    let username = format!("anon_{}", &public_id[..12]);
    let user = db
        .create_user(NewUser {
            username: Some(&username),
            password_hash: None,
            external_id: None,
            user_type: UserType::Internal,
            is_admin: false,
        })
        .await?;
    let key = derive_api_key(secret, public_id);
    db.create_api_key(NewApiKey {
        key_hash: &hash_api_key(&key),
        name: "lightning",
        user_id: user.id,
        last_four: &get_last_four(&key),
        expires_at: None,
    })
    .await?;
    Ok(user.id)
}

async fn topup_user(state: &AppState, headers: &HeaderMap) -> Result<i64, Response> {
    let token = bearer(headers)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Authorization bearer api key is required for topup"))?;
    let key = state
        .db
        .get_api_key_by_hash(&hash_api_key(token))
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Lookup failed"))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "API key not found"))?;
    Ok(key.user_id)
}

fn gateway(state: &AppState) -> Option<Arc<dyn PaymentProvider>> {
    state.payments_state.as_ref().and_then(|p| p.provider())
}

pub async fn create_invoice(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<InvoiceRequest>,
) -> Response {
    let Some(gateway) = gateway(&state) else {
        return error(StatusCode::NOT_FOUND, "Lightning payments not configured on this node");
    };
    if !(MIN_SATS..=MAX_SATS).contains(&body.amount_sats) {
        return error(
            StatusCode::BAD_REQUEST,
            format!("amount_sats must be between {MIN_SATS} and {MAX_SATS}"),
        );
    }

    let secret = match state.db.server_secret().await {
        Ok(s) => s,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Server secret unavailable"),
    };
    let public_id = random_hex();

    let (user_id, created_user) = match body.purpose {
        Purpose::Topup => match topup_user(&state, &headers).await {
            Ok(id) => (id, false),
            Err(response) => return response,
        },
        Purpose::Create => match create_account(&state.db, &secret, &public_id).await {
            Ok(id) => (id, true),
            Err(e) => {
                tracing::error!(error = %e, "Failed to create anonymous account");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create account");
            }
        },
    };

    let created = gateway
        .create_topup(TopupRequest {
            user_id,
            kind: KIND_ANONYMOUS,
            purpose: Some(body.purpose.as_str()),
            currency: SATS,
            amount_milli: body.amount_sats as i64 * 1000,
            lightning: true,
            email: None,
            public_id: Some(&public_id),
        })
        .await;
    let payment = match created {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create gateway payment");
            if created_user {
                let _ = state.db.delete_user(user_id).await;
            }
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create invoice");
        }
    };
    let (Some(bolt11), Some(payment_hash)) = (payment.bolt11.clone(), payment.payment_hash.clone()) else {
        if created_user {
            let _ = state.db.delete_user(user_id).await;
        }
        return error(StatusCode::INTERNAL_SERVER_ERROR, "The payment provider did not return a Lightning invoice");
    };

    (
        StatusCode::OK,
        Json(InvoiceCreated {
            invoice_id: public_id,
            bolt11,
            amount_sats: body.amount_sats as i64,
            expires_at: payment.expires_at.map_or_else(|| chrono::Utc::now().timestamp() + 3600, |t| t.timestamp()),
            payment_hash,
        }),
    )
        .into_response()
}

async fn status_for(state: &AppState, mut row: PublicInvoice) -> Response {
    if row.status == "pending" {
        if let Some(gateway) = gateway(state) {
            if let Ok(Some(payment)) = state.db.gateway_payment(&row.public_id).await {
                let _ = gateway.refresh(&payment).await;
            }
            if let Ok(Some(fresh)) = state.db.public_invoice(&row.public_id).await {
                row = fresh;
            }
        }
    }
    if row.status == "pending" && row.expires_at.is_some_and(|t| chrono::Utc::now() > t) {
        row.status = "expired".to_string();
    }

    let api_key = if row.status == "paid" && row.purpose == "create" {
        state.db.server_secret().await.ok().map(|s| derive_api_key(&s, &row.public_id))
    } else {
        None
    };

    Json(InvoiceStatus {
        status: row.status,
        api_key,
        amount_sats: row.amount_sats,
        paid_at: row.paid_at.map(|t| t.timestamp()),
        created_at: row.created_at.timestamp(),
        expires_at: row.expires_at.map_or(0, |t| t.timestamp()),
    })
    .into_response()
}

pub async fn invoice_status(
    State(state): State<Arc<AppState>>,
    Path(invoice_id): Path<String>,
) -> Response {
    match state.db.public_invoice(&invoice_id).await {
        Ok(Some(row)) => status_for(&state, row).await,
        _ => error(StatusCode::NOT_FOUND, "Invoice not found"),
    }
}

pub async fn recover_invoice(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RecoverRequest>,
) -> Response {
    match state.db.public_invoice_by_bolt11(body.bolt11.trim()).await {
        Ok(Some(row)) => status_for(&state, row).await,
        _ => error(StatusCode::NOT_FOUND, "Invoice not found"),
    }
}

#[cfg(test)]
mod tests;
