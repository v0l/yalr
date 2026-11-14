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
pub struct CreditCurrencyRequest {
    pub user_id: i64,
    pub currency: String,
    pub amount_milli: i64,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct CreditCurrencyResponse {
    pub user_id: i64,
    pub currency: String,
    pub new_balance_milli: i64,
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

pub async fn admin_credit_currency(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreditCurrencyRequest>,
) -> ApiResult<CreditCurrencyResponse> {
    if req.amount_milli <= 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }
    let currency = normalize_currency(&req.currency)
        .ok_or((StatusCode::BAD_REQUEST, "Currency must be a 3 to 8 letter code".to_string()))?;
    if currency == crate::db::SATS {
        return Err((StatusCode::BAD_REQUEST, "Credit sats through /payments/credit".to_string()));
    }
    let payments = state
        .payments_state
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))?;
    let reference = format!("admin-credit-{}", chrono::Utc::now().timestamp_millis());
    let reason = req.reason.as_deref().unwrap_or("admin_credit");
    let new_balance_milli = payments
        .balance_service
        .credit_currency(req.user_id, &currency, req.amount_milli, reason, &reference)
        .await
        .map_err(internal)?;
    Ok(Json(CreditCurrencyResponse { user_id: req.user_id, currency, new_balance_milli }))
}
