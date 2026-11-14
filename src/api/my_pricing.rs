use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::auth::admin::UserExtractor;
use crate::db::{FiatPrices, ModelPricingRow, NewModelPricing, UpdateModelPricing, SATS};
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Serialize)]
pub struct PricingView {
    pub id: i64,
    pub model_name: String,
    pub is_advertised: bool,
    pub is_free: bool,
    pub currency: String,
    pub price_per_1m_input: Option<i64>,
    pub price_per_1m_output: Option<i64>,
    pub price_per_1m_cache_read: Option<i64>,
    pub price_per_1m_cache_write: Option<i64>,
    pub price_per_request: Option<i64>,
    pub owner_user_id: Option<i64>,
    pub owner_username: Option<String>,
    pub provider_id: Option<i64>,
    pub provider_slug: Option<String>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
}

pub async fn view(state: &AppState, row: ModelPricingRow) -> PricingView {
    let sats = row.currency == SATS;
    let pick = |s: Option<i64>, m: Option<i64>| if sats { s } else { m };
    let owner_username = match row.owner_user_id {
        Some(id) => state.db.provider_owner_username(id).await.ok().flatten(),
        None => None,
    };
    let provider_slug = match row.provider_id {
        Some(id) => state.db.get_provider_by_id(id).await.ok().flatten().map(|p| p.slug),
        None => None,
    };
    PricingView {
        id: row.id,
        model_name: row.model_name,
        is_advertised: row.is_advertised,
        is_free: row.is_free,
        price_per_1m_input: pick(row.price_per_1m_input_sats, row.price_per_1m_input_milli),
        price_per_1m_output: pick(row.price_per_1m_output_sats, row.price_per_1m_output_milli),
        price_per_1m_cache_read: pick(row.price_per_1m_cache_read_sats, row.price_per_1m_cache_read_milli),
        price_per_1m_cache_write: pick(row.price_per_1m_cache_write_sats, row.price_per_1m_cache_write_milli),
        price_per_request: pick(row.price_per_request_sats, row.price_per_request_milli),
        currency: row.currency,
        owner_user_id: row.owner_user_id,
        owner_username,
        provider_id: row.provider_id,
        provider_slug,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

#[derive(Deserialize)]
pub struct PriceInput {
    pub currency: String,
    pub price_per_1m_input: Option<i64>,
    pub price_per_1m_output: Option<i64>,
    pub price_per_1m_cache_read: Option<i64>,
    pub price_per_1m_cache_write: Option<i64>,
    pub price_per_request: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateMyPrice {
    pub provider_slug: String,
    pub model_name: String,
    #[serde(flatten)]
    pub price: PriceInput,
    #[serde(default = "yes")]
    pub is_advertised: bool,
}

fn yes() -> bool {
    true
}

fn internal(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn currency_of(raw: &str) -> Result<String, (StatusCode, String)> {
    crate::db::normalize_currency(raw)
        .ok_or((StatusCode::BAD_REQUEST, "Currency must be sats or a 3 to 8 letter code".to_string()))
}

fn non_negative(price: &PriceInput) -> Result<(), (StatusCode, String)> {
    let all = [
        price.price_per_1m_input,
        price.price_per_1m_output,
        price.price_per_1m_cache_read,
        price.price_per_1m_cache_write,
        price.price_per_request,
    ];
    if all.iter().flatten().any(|v| *v < 0) {
        return Err((StatusCode::BAD_REQUEST, "Prices cannot be negative".to_string()));
    }
    Ok(())
}

fn split(price: &PriceInput, currency: &str) -> ([Option<i64>; 5], FiatPrices) {
    let values = [
        price.price_per_1m_input,
        price.price_per_1m_output,
        price.price_per_1m_cache_read,
        price.price_per_1m_cache_write,
        price.price_per_request,
    ];
    if currency == SATS {
        (values, FiatPrices::default())
    } else {
        (
            [None; 5],
            FiatPrices {
                input: values[0],
                output: values[1],
                cache_read: values[2],
                cache_write: values[3],
                request: values[4],
            },
        )
    }
}

pub async fn list(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> ApiResult<Vec<PricingView>> {
    let rows = state.db.list_model_pricings_owned_by(auth.user.id).await.map_err(internal)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(view(&state, row).await);
    }
    Ok(Json(out))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Json(req): Json<CreateMyPrice>,
) -> ApiResult<PricingView> {
    let currency = currency_of(&req.price.currency)?;
    non_negative(&req.price)?;
    let provider = crate::api::my_providers::owned_provider(&state, &auth.user, &req.provider_slug).await?;
    let owner = provider.owner_user_id.ok_or((StatusCode::FORBIDDEN, "Platform providers are priced by admins".to_string()))?;
    let (sats, fiat) = split(&req.price, &currency);
    let row = state
        .db
        .create_model_pricing(NewModelPricing {
            model_name: req.model_name.trim(),
            is_advertised: req.is_advertised,
            is_free: false,
            price_per_1m_input_sats: sats[0],
            price_per_1m_output_sats: sats[1],
            price_per_1m_cache_read_sats: sats[2],
            price_per_1m_cache_write_sats: sats[3],
            price_per_request_sats: sats[4],
            context_window: None,
            max_output_tokens: None,
            owner_user_id: Some(owner),
            provider_id: Some(provider.id),
            currency: &currency,
            fiat,
        })
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db) if db.is_unique_violation() => {
                (StatusCode::CONFLICT, "That provider already has a price for this model".to_string())
            }
            other => internal(other),
        })?;
    Ok(Json(view(&state, row).await))
}

async fn owned_row(
    state: &AppState,
    user: &crate::db::User,
    id: i64,
) -> Result<ModelPricingRow, (StatusCode, String)> {
    let row = state
        .db
        .model_pricing_by_id(id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Price not found".to_string()))?;
    if row.owner_user_id == Some(user.id) || (user.is_admin && row.owner_user_id.is_some()) {
        Ok(row)
    } else {
        Err((StatusCode::NOT_FOUND, "Price not found".to_string()))
    }
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(id): Path<i64>,
    Json(req): Json<PriceInput>,
) -> ApiResult<PricingView> {
    let row = owned_row(&state, &auth.user, id).await?;
    let currency = currency_of(&req.currency)?;
    non_negative(&req)?;
    let (sats, fiat) = split(&req, &currency);
    let updated = state
        .db
        .update_pricing_row(
            row.id,
            UpdateModelPricing {
                is_advertised: None,
                is_free: None,
                price_per_1m_input_sats: Some(sats[0]),
                price_per_1m_output_sats: Some(sats[1]),
                price_per_1m_cache_read_sats: Some(sats[2]),
                price_per_1m_cache_write_sats: Some(sats[3]),
                price_per_request_sats: Some(sats[4]),
                context_window: None,
                max_output_tokens: None,
                currency: Some(currency),
                fiat: Some(fiat),
            },
        )
        .await
        .map_err(internal)?;
    Ok(Json(view(&state, updated).await))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(id): Path<i64>,
) -> ApiResult<bool> {
    let row = owned_row(&state, &auth.user, id).await?;
    state.db.delete_pricing_row(row.id).await.map_err(internal)?;
    Ok(Json(true))
}
