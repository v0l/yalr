use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::tenant::{bad_request, internal, not_found, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::payouts::{PayoutError, PayoutRow};
use crate::db::{milli_per_unit, normalize_currency};
use crate::payouts::revolut::{RevolutBusiness, WebhookEvent};
use crate::payouts::service::{PayoutRequest, PayoutService, RequestError};
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, TenantError>;

#[derive(Serialize)]
pub struct CurrencyWithdrawal {
    pub currency: String,
    pub withdrawable_milli: i64,
    pub min_milli: i64,
    pub daily_cap_milli: i64,
    pub milli_per_unit: i64,
    pub methods: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct WithdrawalOverview {
    pub enabled: bool,
    pub hold_days: i64,
    pub currencies: Vec<CurrencyWithdrawal>,
    pub payouts: Vec<PayoutRow>,
}

#[derive(Deserialize)]
pub struct WithdrawRequest {
    pub currency: String,
    pub amount_milli: i64,
    pub method: String,
    pub destination: String,
}

#[derive(Deserialize)]
pub struct StatusFilter {
    pub status: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct RejectRequest {
    #[serde(default)]
    pub reason: String,
}

fn payout_error(e: PayoutError) -> TenantError {
    match e {
        PayoutError::NotFound => not_found("Payout not found"),
        PayoutError::WrongState => (StatusCode::CONFLICT, "Payout is not in a state that allows this".into()),
        PayoutError::Duplicate => (StatusCode::CONFLICT, "That destination was already used for a payout".into()),
        PayoutError::BelowMinimum { min } => bad_request(&format!("Below the minimum payout of {min}")),
        PayoutError::OverDailyCap { remaining } => bad_request(&format!("Over the daily limit, {remaining} left today")),
        PayoutError::NotWithdrawable { withdrawable } => {
            bad_request(&format!("Only matured earnings can be withdrawn, {withdrawable} available"))
        }
        PayoutError::Db(m) => internal(m),
    }
}

fn request_error(e: RequestError) -> TenantError {
    match e {
        RequestError::Disabled => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
        RequestError::MethodUnavailable(_) | RequestError::Destination(_) => bad_request(&e.to_string()),
        RequestError::Rejected(inner) => payout_error(inner),
    }
}

const CURRENCIES: [&str; 3] = [crate::db::SATS, "eur", "usd"];

pub async fn overview(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> ApiResult<WithdrawalOverview> {
    let service = PayoutService::from_state(&state);
    let config = service.config();
    let mut currencies = Vec::new();
    for currency in CURRENCIES {
        let methods = config.methods_for(currency);
        if methods.is_empty() {
            continue;
        }
        let rules = config.rules(currency);
        currencies.push(CurrencyWithdrawal {
            currency: currency.to_string(),
            withdrawable_milli: service.withdrawable(auth.user.id, currency).await.map_err(internal)?,
            min_milli: rules.min_amount,
            daily_cap_milli: rules.daily_cap,
            milli_per_unit: milli_per_unit(currency),
            methods,
        });
    }
    let payouts = state.db.list_user_payouts(auth.user.id).await.map_err(internal)?;
    Ok(Json(WithdrawalOverview { enabled: config.enabled, hold_days: config.hold_days, currencies, payouts }))
}

pub async fn withdraw(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Json(req): Json<WithdrawRequest>,
) -> ApiResult<PayoutRow> {
    let currency = normalize_currency(&req.currency).ok_or(bad_request("Currency must be a 3 to 8 letter code"))?;
    let row = PayoutService::from_state(&state)
        .request(PayoutRequest {
            user_id: auth.user.id,
            currency: &currency,
            amount: req.amount_milli,
            method: &req.method,
            destination: &req.destination,
        })
        .await
        .map_err(request_error)?;
    Ok(Json(row))
}

pub async fn cancel_mine(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(id): Path<i64>,
) -> ApiResult<PayoutRow> {
    let row = PayoutService::from_state(&state).cancel(id, Some(auth.user.id)).await.map_err(payout_error)?;
    Ok(Json(row))
}

pub async fn list(State(state): State<Arc<AppState>>, Query(filter): Query<StatusFilter>) -> ApiResult<Vec<PayoutRow>> {
    Ok(Json(state.db.list_payouts(filter.status.as_deref()).await.map_err(internal)?))
}

pub async fn approve(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> ApiResult<PayoutRow> {
    Ok(Json(PayoutService::from_state(&state).approve(id).await.map_err(payout_error)?))
}

pub async fn reject(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    body: Option<Json<RejectRequest>>,
) -> ApiResult<PayoutRow> {
    let reason = body.map(|b| b.0.reason).filter(|r| !r.is_empty()).unwrap_or_else(|| "rejected by admin".into());
    Ok(Json(state.db.reject_payout(id, &reason).await.map_err(payout_error)?))
}

pub async fn retry(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> ApiResult<PayoutRow> {
    Ok(Json(PayoutService::from_state(&state).retry(id).await.map_err(payout_error)?))
}

pub async fn admin_cancel(State(state): State<Arc<AppState>>, Path(id): Path<i64>) -> ApiResult<PayoutRow> {
    Ok(Json(PayoutService::from_state(&state).cancel(id, None).await.map_err(payout_error)?))
}

pub async fn revolut_webhook(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> StatusCode {
    let config = crate::payouts::settings::get();
    let Some(revolut) = config.revolut.as_ref().map(RevolutBusiness::new) else {
        return StatusCode::NOT_FOUND;
    };
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    if !revolut.verify_webhook(&header("revolut-signature"), &header("revolut-request-timestamp"), &body, now_ms) {
        return StatusCode::UNAUTHORIZED;
    }
    if let Ok(event) = serde_json::from_slice::<WebhookEvent>(&body) {
        if event.event == "PayoutLinkStateChanged" {
            PayoutService::from_state(&state).handle_webhook(&event.data.id).await;
        }
    }
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use crate::api::tenant_tests::tenancy;
    use crate::payouts::config::PayoutsConfig;

    #[tokio::test]
    async fn payout_routes_are_scoped_to_the_user_and_admin_gated() {
        crate::payouts::settings::set(PayoutsConfig { enabled: true, min_sats: 1, hold_days: 0, ..PayoutsConfig::default() });
        let t = tenancy().await;
        let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
        t.state.db.credit_currency(alice.id, "sats", 50_000_000, "provider_earning", "e", 0).await.unwrap();

        let (status, body) = t.get(&t.alice, "/api/my/payouts").await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["currencies"][0]["currency"], "sats");
        assert_eq!(body["currencies"][0]["withdrawable_milli"], 50_000_000);

        let (status, _) = t.call(&t.alice, "POST", "/api/my/payouts", r#"{"currency":"sats","amount_milli":1000000,"method":"lightning","destination":"nonsense"}"#).await;
        assert_eq!(status, 400);
        let (status, _) = t.call(&t.alice, "POST", "/api/my/payouts", r#"{"currency":"eur","amount_milli":1000000,"method":"revolut_link","destination":"Alice"}"#).await;
        assert_eq!(status, 400);

        assert_eq!(t.get(&t.alice, "/api/payouts").await.0, 403);
        assert_eq!(t.call(&t.alice, "POST", "/api/payouts/1/approve", "").await.0, 403);
        assert_eq!(t.get(&t.admin, "/api/payouts").await.0, 200);

        let bob = t.state.db.get_user_by_username("bob").await.unwrap().unwrap();
        let id = t
            .state
            .db
            .execute_raw(&format!(
                "INSERT INTO payouts (user_id, currency, amount, method, destination, status) VALUES ({}, 'sats', 1000, 'lightning', 'x', 'requested')",
                bob.id
            ))
            .await
            .unwrap();
        assert_eq!(id, 1);
        let (status, _) = t.call(&t.alice, "POST", "/api/my/payouts/1/cancel", "").await;
        assert_eq!(status, 404);
        let (_, mine) = t.get(&t.alice, "/api/my/payouts").await;
        assert!(mine["payouts"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn revolut_business_webhook_rejects_unsigned_calls() {
        let t = tenancy().await;
        let (status, _) = t.call("", "POST", "/api/webhooks/revolut-business", "{}").await;
        assert!(status == 401 || status == 404, "{status}");
    }
}
