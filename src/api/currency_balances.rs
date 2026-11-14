use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::auth::admin::UserExtractor;
use crate::db::normalize_currency;
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Serialize)]
pub struct CurrencyBalance {
    pub currency: String,
    pub balance_milli: i64,
    pub lifetime_deposited_milli: i64,
}

#[derive(Serialize)]
pub struct Balances {
    pub user_id: i64,
    pub balance_msat: i64,
    pub lifetime_deposited_msat: i64,
    pub currencies: Vec<CurrencyBalance>,
}

#[derive(Deserialize)]
pub struct AdjustBalanceRequest {
    pub user_id: i64,
    #[serde(default = "default_currency")]
    pub currency: String,
    pub amount_milli: i64,
    pub reason: Option<String>,
}

fn default_currency() -> String {
    crate::db::SATS.to_string()
}

#[derive(Serialize)]
pub struct AdjustBalanceResponse {
    pub success: bool,
    pub user_id: i64,
    pub currency: String,
    pub amount_milli: i64,
    pub new_balance_milli: i64,
    pub reason: String,
}

#[derive(Serialize)]
pub struct CurrencyOption {
    pub code: String,
    pub milli_per_unit: i64,
}

fn internal(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn balances_of(state: &AppState, user_id: i64) -> Result<Balances, (StatusCode, String)> {
    let sats = state.db.get_user_balance(user_id).await.map_err(internal)?;
    let currencies = state
        .db
        .list_currency_balances(user_id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|(currency, balance_milli, lifetime_deposited_milli)| CurrencyBalance {
            currency,
            balance_milli,
            lifetime_deposited_milli,
        })
        .collect();
    Ok(Balances {
        user_id,
        balance_msat: sats.as_ref().map_or(0, |b| b.balance_msat),
        lifetime_deposited_msat: sats.as_ref().map_or(0, |b| b.lifetime_deposited_msat),
        currencies,
    })
}

pub async fn my_balances(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> ApiResult<Balances> {
    Ok(Json(balances_of(&state, auth.user.id).await?))
}

pub async fn user_balances(State(state): State<Arc<AppState>>, Path(user_id): Path<i64>) -> ApiResult<Balances> {
    Ok(Json(balances_of(&state, user_id).await?))
}

struct Adjustment {
    currency: String,
    amount_milli: i64,
    reason: String,
}

fn adjustment(req: &AdjustBalanceRequest, default_reason: &str) -> Result<Adjustment, (StatusCode, String)> {
    if req.amount_milli <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }
    let currency = normalize_currency(&req.currency)
        .ok_or((StatusCode::BAD_REQUEST, "Currency must be a 3 to 8 letter code".to_string()))?;
    let reason = req.reason.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| default_reason.to_string());
    Ok(Adjustment { currency, amount_milli: req.amount_milli, reason })
}

fn adjusted(req: &AdjustBalanceRequest, adjustment: Adjustment, new_balance_milli: i64) -> AdjustBalanceResponse {
    AdjustBalanceResponse {
        success: true,
        user_id: req.user_id,
        currency: adjustment.currency,
        amount_milli: adjustment.amount_milli,
        new_balance_milli,
        reason: adjustment.reason,
    }
}

fn payments(state: &AppState) -> Result<&Arc<crate::payments::PaymentsState>, (StatusCode, String)> {
    state
        .payments_state
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))
}

pub async fn admin_credit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdjustBalanceRequest>,
) -> ApiResult<AdjustBalanceResponse> {
    let adjustment = adjustment(&req, "admin_credit")?;
    let reference = format!("admin-credit-{}", chrono::Utc::now().timestamp_millis());
    let balance = payments(&state)?
        .balance_service
        .credit_currency(req.user_id, &adjustment.currency, adjustment.amount_milli, &adjustment.reason, &reference)
        .await
        .map_err(internal)?;
    tracing::info!(admin_action = "credit", user_id = req.user_id, currency = %adjustment.currency, amount_milli = adjustment.amount_milli, "Admin manual credit");
    Ok(Json(adjusted(&req, adjustment, balance)))
}

pub async fn admin_debit(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdjustBalanceRequest>,
) -> ApiResult<AdjustBalanceResponse> {
    let adjustment = adjustment(&req, "admin_debit")?;
    let reference = format!("admin-debit-{}", chrono::Utc::now().timestamp_millis());
    let balance = payments(&state)?
        .balance_service
        .debit_currency(req.user_id, &adjustment.currency, adjustment.amount_milli, &adjustment.reason, &reference)
        .await
        .map_err(|e| match e {
            crate::payments::balance::BalanceError::InsufficientFunds { .. } => (StatusCode::BAD_REQUEST, e.to_string()),
            other => internal(other),
        })?;
    tracing::info!(admin_action = "debit", user_id = req.user_id, currency = %adjustment.currency, amount_milli = adjustment.amount_milli, "Admin manual debit");
    Ok(Json(adjusted(&req, adjustment, balance)))
}

pub async fn currencies(State(state): State<Arc<AppState>>) -> ApiResult<Vec<CurrencyOption>> {
    let codes = state.db.known_currencies().await.map_err(internal)?;
    Ok(Json(
        codes
            .into_iter()
            .map(|code| CurrencyOption { milli_per_unit: crate::db::milli_per_unit(&code), code })
            .collect(),
    ))
}
