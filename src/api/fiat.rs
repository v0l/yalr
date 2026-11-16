use crate::api::tenant::{bad_request, internal, not_found, TenantError};
use crate::auth::admin::{AdminExtractor, UserExtractor};
use crate::db::auto_reload::{AutoReload, AutoReloadSettings};
use crate::db::fiat_orders::{FiatOrder, FiatOrderWithUser};
use crate::db::normalize_currency;
use crate::payments::revolut::{RevolutError, RevolutService};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use payments_rs::fiat::{RevolutWebhookBody, RevolutWebhookEvent};
use payments_rs::webhook::WebhookMessage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const HISTORY_LIMIT: i64 = 100;
const MILLI_PER_UNIT: f64 = 100_000.0;

fn service(state: &AppState) -> Result<Arc<RevolutService>, TenantError> {
    state
        .payments_state
        .as_ref()
        .and_then(|p| p.revolut.clone())
        .ok_or_else(|| not_found("Card top-ups are not enabled"))
}

fn failed(e: RevolutError) -> TenantError {
    match e {
        RevolutError::Invalid(m) => bad_request(&m),
        RevolutError::Upstream(m) => {
            tracing::warn!(error = %m, "Revolut request failed");
            (StatusCode::BAD_GATEWAY, "The card processor could not be reached".to_string())
        }
        RevolutError::Database(e) => internal(e),
    }
}

#[derive(Serialize)]
pub struct FiatConfig {
    pub mode: &'static str,
    pub currencies: Vec<String>,
    pub min_amount: i64,
    pub max_amount: i64,
}

pub async fn config(State(state): State<Arc<AppState>>, _user: UserExtractor) -> Result<Json<FiatConfig>, TenantError> {
    let s = service(&state)?;
    Ok(Json(FiatConfig {
        mode: s.mode(),
        currencies: s.settings.currencies.clone(),
        min_amount: s.settings.min_amount,
        max_amount: s.settings.max_amount,
    }))
}

#[derive(Deserialize)]
pub struct CreateTopup {
    pub currency: String,
    pub amount: f64,
    #[serde(default)]
    pub save_card: bool,
    pub email: Option<String>,
}

#[derive(Serialize)]
pub struct CreatedTopup {
    pub id: String,
    pub token: String,
    pub mode: &'static str,
    pub currency: String,
    pub amount_milli: i64,
}

pub async fn create_topup(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Json(req): Json<CreateTopup>,
) -> Result<Json<CreatedTopup>, TenantError> {
    let s = service(&state)?;
    let currency = normalize_currency(&req.currency).filter(|c| s.currency_allowed(c)).ok_or_else(|| bad_request("Unsupported currency"))?;
    let minor = s.check_amount(req.amount).map_err(failed)?;
    let created = s.create_topup(auth.user.id, &currency, minor, req.save_card, req.email).await.map_err(failed)?;
    Ok(Json(CreatedTopup {
        id: created.order.public_id,
        token: created.token,
        mode: s.mode(),
        currency,
        amount_milli: created.order.amount_milli,
    }))
}

pub async fn topup_status(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(id): Path<String>,
) -> Result<Json<FiatOrder>, TenantError> {
    let s = service(&state)?;
    let order = state.db.fiat_order(&id).await.map_err(internal)?.filter(|o| o.user_id == auth.user.id).ok_or_else(|| not_found("Order not found"))?;
    if order.status == crate::db::fiat_orders::PENDING {
        if let Some(external) = order.external_id.as_deref() {
            let _ = s.reconcile(external).await;
            if let Some(fresh) = state.db.fiat_order(&id).await.map_err(internal)? {
                return Ok(Json(fresh));
            }
        }
    }
    Ok(Json(order))
}

pub async fn history(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> Result<Json<Vec<FiatOrder>>, TenantError> {
    service(&state)?;
    Ok(Json(state.db.list_user_fiat_orders(auth.user.id, HISTORY_LIMIT).await.map_err(internal)?))
}

pub async fn admin_orders(State(state): State<Arc<AppState>>, _admin: AdminExtractor) -> Result<Json<Vec<FiatOrderWithUser>>, TenantError> {
    service(&state)?;
    Ok(Json(state.db.list_all_fiat_orders(500).await.map_err(internal)?))
}

#[derive(Serialize)]
pub struct SavedCard {
    pub id: i64,
    pub brand: Option<String>,
    pub last_four: Option<String>,
    pub exp_month: Option<u16>,
    pub exp_year: Option<u16>,
    pub is_default: bool,
}

pub async fn list_cards(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> Result<Json<Vec<SavedCard>>, TenantError> {
    service(&state)?;
    let cards = state.db.list_payment_methods(auth.user.id).await.map_err(internal)?;
    Ok(Json(
        cards
            .into_iter()
            .map(|c| SavedCard { id: c.id, brand: c.card_brand, last_four: c.card_last_four, exp_month: c.exp_month, exp_year: c.exp_year, is_default: c.is_default })
            .collect(),
    ))
}

pub async fn delete_card(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor, Path(id): Path<i64>) -> Result<StatusCode, TenantError> {
    let s = service(&state)?;
    if s.remove_card(auth.user.id, id).await.map_err(failed)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found("Card not found"))
    }
}

pub async fn default_card(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor, Path(id): Path<i64>) -> Result<StatusCode, TenantError> {
    service(&state)?;
    if state.db.set_default_payment_method(auth.user.id, id).await.map_err(internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found("Card not found"))
    }
}

#[derive(Serialize)]
pub struct AutoReloadView {
    pub currency: String,
    pub enabled: bool,
    pub threshold_milli: i64,
    pub amount_milli: i64,
    pub monthly_cap_milli: i64,
    pub payment_method_id: i64,
    pub consecutive_failures: i32,
    pub disabled_reason: Option<String>,
}

impl From<AutoReload> for AutoReloadView {
    fn from(a: AutoReload) -> Self {
        Self {
            currency: a.currency,
            enabled: a.enabled,
            threshold_milli: a.threshold_milli,
            amount_milli: a.amount_milli,
            monthly_cap_milli: a.monthly_cap_milli,
            payment_method_id: a.payment_method_id,
            consecutive_failures: a.consecutive_failures,
            disabled_reason: a.disabled_reason,
        }
    }
}

pub async fn list_auto_reload(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> Result<Json<Vec<AutoReloadView>>, TenantError> {
    service(&state)?;
    let rows = state.db.list_auto_reloads(auth.user.id).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
pub struct SaveAutoReload {
    pub currency: String,
    pub enabled: bool,
    pub threshold: f64,
    pub amount: f64,
    pub monthly_cap: f64,
    pub payment_method_id: i64,
}

pub async fn save_auto_reload(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Json(req): Json<SaveAutoReload>,
) -> Result<Json<AutoReloadView>, TenantError> {
    let s = service(&state)?;
    let currency = normalize_currency(&req.currency).filter(|c| s.currency_allowed(c)).ok_or_else(|| bad_request("Unsupported currency"))?;
    let amount_minor = s.check_amount(req.amount).map_err(failed)?;
    if req.threshold <= 0.0 || req.monthly_cap < req.amount {
        return Err(bad_request("Threshold must be positive and the monthly cap at least one reload"));
    }
    state.db.payment_method(auth.user.id, req.payment_method_id).await.map_err(internal)?.ok_or_else(|| not_found("Card not found"))?;
    let saved = state
        .db
        .save_auto_reload(AutoReloadSettings {
            user_id: auth.user.id,
            currency: &currency,
            enabled: req.enabled,
            threshold_milli: (req.threshold * MILLI_PER_UNIT).round() as i64,
            amount_milli: amount_minor * 1_000,
            monthly_cap_milli: (req.monthly_cap * MILLI_PER_UNIT).round() as i64,
            payment_method_id: req.payment_method_id,
        })
        .await
        .map_err(internal)?;
    Ok(Json(saved.into()))
}

pub async fn webhook(State(state): State<Arc<AppState>>, headers: HeaderMap, body: axum::body::Bytes) -> StatusCode {
    let Ok(s) = service(&state) else { return StatusCode::NOT_FOUND };
    let message = WebhookMessage {
        endpoint: "/api/webhooks/revolut".to_string(),
        body: body.to_vec(),
        headers: headers
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string())))
            .collect(),
    };
    let event = match RevolutWebhookBody::verify(s.webhook_secret(), &message) {
        Ok(event) => event,
        Err(e) => {
            tracing::warn!(error = %e, "Rejected Revolut webhook");
            return StatusCode::UNAUTHORIZED;
        }
    };
    if matches!(event.event, RevolutWebhookEvent::Unknown) {
        return StatusCode::OK;
    }
    match s.reconcile(&event.order_id).await {
        Ok(_) => StatusCode::OK,
        Err(e) => {
            tracing::warn!(order_id = %event.order_id, error = %e, "Revolut webhook could not be applied");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
