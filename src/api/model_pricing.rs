use crate::api::tenant::{bad_request, forbidden, internal, not_found, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{FiatPrices, ModelPricingRow, SATS};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ModelPricingResponse {
    pub id: i64,
    pub model_name: String,
    pub is_advertised: bool,
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_1m_cache_read_sats: Option<i64>,
    pub price_per_1m_cache_write_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub currency: String,
    pub price_per_1m_input_milli: Option<i64>,
    pub price_per_1m_output_milli: Option<i64>,
    pub price_per_1m_cache_read_milli: Option<i64>,
    pub price_per_1m_cache_write_milli: Option<i64>,
    pub price_per_request_milli: Option<i64>,
    pub owner_user_id: Option<i64>,
    pub owner_username: Option<String>,
    pub provider_id: Option<i64>,
    pub provider_slug: Option<String>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
}

#[derive(Deserialize)]
pub struct ModelPricingCreateRequest {
    pub model_name: String,
    #[serde(default = "default_true")]
    pub is_advertised: bool,
    #[serde(default)]
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_1m_cache_read_sats: Option<i64>,
    pub price_per_1m_cache_write_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    #[serde(default)]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(flatten)]
    pub fiat: FiatInput,
}

#[derive(Deserialize, Default)]
pub struct FiatInput {
    pub price_per_1m_input_milli: Option<i64>,
    pub price_per_1m_output_milli: Option<i64>,
    pub price_per_1m_cache_read_milli: Option<i64>,
    pub price_per_1m_cache_write_milli: Option<i64>,
    pub price_per_request_milli: Option<i64>,
}

impl FiatInput {
    fn prices(&self) -> FiatPrices {
        FiatPrices {
            input: self.price_per_1m_input_milli,
            output: self.price_per_1m_output_milli,
            cache_read: self.price_per_1m_cache_read_milli,
            cache_write: self.price_per_1m_cache_write_milli,
            request: self.price_per_request_milli,
        }
    }
}

#[derive(Deserialize)]
pub struct ModelPricingUpdateRequest {
    pub is_advertised: Option<bool>,
    pub is_free: Option<bool>,
    pub price_per_1m_input_sats: Option<Option<i64>>,
    pub price_per_1m_output_sats: Option<Option<i64>>,
    pub price_per_1m_cache_read_sats: Option<Option<i64>>,
    pub price_per_1m_cache_write_sats: Option<Option<i64>>,
    pub price_per_request_sats: Option<Option<i64>>,
    pub context_window: Option<Option<i32>>,
    pub max_output_tokens: Option<Option<i32>>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default, flatten)]
    pub fiat: Option<FiatInput>,
}

#[derive(Serialize)]
pub struct Deleted {
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
}

fn default_true() -> bool { true }

pub async fn response(state: &AppState, r: crate::db::ModelPricingRow) -> ModelPricingResponse {
    let owner_username = match r.owner_user_id {
        Some(id) => state.db.provider_owner_username(id).await.ok().flatten(),
        None => None,
    };
    let provider_slug = match r.provider_id {
        Some(id) => state.db.get_provider_by_id(id).await.ok().flatten().map(|p| p.slug),
        None => None,
    };
    ModelPricingResponse {
        id: r.id,
        model_name: r.model_name,
        is_advertised: r.is_advertised,
        is_free: r.is_free,
        price_per_1m_input_sats: r.price_per_1m_input_sats,
        price_per_1m_output_sats: r.price_per_1m_output_sats,
        price_per_1m_cache_read_sats: r.price_per_1m_cache_read_sats,
        price_per_1m_cache_write_sats: r.price_per_1m_cache_write_sats,
        price_per_request_sats: r.price_per_request_sats,
        context_window: r.context_window,
        max_output_tokens: r.max_output_tokens,
        currency: r.currency,
        price_per_1m_input_milli: r.price_per_1m_input_milli,
        price_per_1m_output_milli: r.price_per_1m_output_milli,
        price_per_1m_cache_read_milli: r.price_per_1m_cache_read_milli,
        price_per_1m_cache_write_milli: r.price_per_1m_cache_write_milli,
        price_per_request_milli: r.price_per_request_milli,
        owner_user_id: r.owner_user_id,
        owner_username,
        provider_id: r.provider_id,
        provider_slug,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

fn currency_of(raw: Option<&str>) -> Result<String, TenantError> {
    match raw {
        None => Ok(SATS.to_string()),
        Some(code) => crate::db::normalize_currency(code)
            .ok_or_else(|| bad_request("Currency must be sats or a 3 to 8 letter code")),
    }
}

fn no_negatives(values: &[Option<i64>]) -> Result<(), TenantError> {
    if values.iter().flatten().any(|v| *v < 0) {
        return Err(bad_request("Prices cannot be negative"));
    }
    Ok(())
}

async fn visible_row(scope: &Scope, state: &AppState, id: i64) -> Result<ModelPricingRow, TenantError> {
    state
        .db
        .model_pricing_by_id(id)
        .await
        .ok()
        .filter(|row| scope.sees(row.owner_user_id))
        .ok_or_else(|| not_found("Model pricing not found"))
}

pub async fn list_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<Vec<ModelPricingResponse>>, TenantError> {
    let scope = Scope::from(user);
    let rows = match scope.owner() {
        None => state.db.list_model_pricings().await,
        Some(owner) => state.db.list_model_pricings_owned_by(owner).await,
    }
    .map_err(internal)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(response(&state, row).await);
    }
    Ok(Json(out))
}

pub async fn create_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
    Json(req): Json<ModelPricingCreateRequest>,
) -> Result<Json<ModelPricingResponse>, TenantError> {
    let scope = Scope::from(user);
    let provider = match req.provider_slug.as_deref().filter(|s| !s.is_empty()) {
        Some(slug) => Some(scope.provider(&state, slug).await?),
        None if scope.is_admin() => None,
        None => return Err(bad_request("Choose one of your providers to price")),
    };
    let owner_user_id = provider.as_ref().and_then(|p| p.owner_user_id);
    if !scope.is_admin() && owner_user_id != Some(scope.user.id) {
        return Err(forbidden("You can only price your own providers"));
    }
    let currency = currency_of(req.currency.as_deref())?;
    let fiat = if currency == SATS { FiatPrices::default() } else { req.fiat.prices() };
    let sats = |v: Option<i64>| if currency == SATS { v } else { None };
    no_negatives(&[
        req.price_per_1m_input_sats,
        req.price_per_1m_output_sats,
        req.price_per_1m_cache_read_sats,
        req.price_per_1m_cache_write_sats,
        req.price_per_request_sats,
        fiat.input,
        fiat.output,
        fiat.cache_read,
        fiat.cache_write,
        fiat.request,
    ])?;
    let mp = crate::db::NewModelPricing {
        model_name: req.model_name.trim(),
        is_advertised: req.is_advertised,
        is_free: req.is_free && scope.is_admin(),
        price_per_1m_input_sats: sats(req.price_per_1m_input_sats),
        price_per_1m_output_sats: sats(req.price_per_1m_output_sats),
        price_per_1m_cache_read_sats: sats(req.price_per_1m_cache_read_sats),
        price_per_1m_cache_write_sats: sats(req.price_per_1m_cache_write_sats),
        price_per_request_sats: sats(req.price_per_request_sats),
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
        owner_user_id,
        provider_id: provider.as_ref().map(|p| p.id),
        currency: &currency,
        fiat,
    };
    let row = state.db.create_model_pricing(mp).await.map_err(|e| match e {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            (StatusCode::CONFLICT, "That model already has a price for this provider".to_string())
        }
        other => internal(other),
    })?;
    Ok(Json(response(&state, row).await))
}

fn updates(req: ModelPricingUpdateRequest, is_admin: bool) -> Result<crate::db::UpdateModelPricing, TenantError> {
    let currency = req.currency.as_deref().map(|c| currency_of(Some(c))).transpose()?;
    let fiat = req.fiat.as_ref().map(FiatInput::prices);
    if let Some(f) = &fiat {
        no_negatives(&[f.input, f.output, f.cache_read, f.cache_write, f.request])?;
    }
    no_negatives(&[
        req.price_per_1m_input_sats.flatten(),
        req.price_per_1m_output_sats.flatten(),
        req.price_per_1m_cache_read_sats.flatten(),
        req.price_per_1m_cache_write_sats.flatten(),
        req.price_per_request_sats.flatten(),
    ])?;
    Ok(crate::db::UpdateModelPricing {
        is_advertised: req.is_advertised,
        is_free: req.is_free.filter(|_| is_admin),
        price_per_1m_input_sats: req.price_per_1m_input_sats,
        price_per_1m_output_sats: req.price_per_1m_output_sats,
        price_per_1m_cache_read_sats: req.price_per_1m_cache_read_sats,
        price_per_1m_cache_write_sats: req.price_per_1m_cache_write_sats,
        price_per_request_sats: req.price_per_request_sats,
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
        currency,
        fiat,
    })
}

pub async fn update_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
    Json(req): Json<ModelPricingUpdateRequest>,
) -> Result<Json<ModelPricingResponse>, TenantError> {
    let scope = Scope::from(user);
    if !scope.is_admin() {
        return Err(forbidden("Edit your prices by id"));
    }
    let row = state.db.update_model_pricing(&model_name, updates(req, true)?).await.map_err(|e| match e {
        sqlx::Error::RowNotFound => not_found("Model pricing not found"),
        other => internal(other),
    })?;
    Ok(Json(response(&state, row).await))
}

pub async fn update_pricing_by_id(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
    Json(req): Json<ModelPricingUpdateRequest>,
) -> Result<Json<ModelPricingResponse>, TenantError> {
    let scope = Scope::from(user);
    let row = visible_row(&scope, &state, id).await?;
    let updated = state.db.update_pricing_row(row.id, updates(req, scope.is_admin())?).await.map_err(internal)?;
    Ok(Json(response(&state, updated).await))
}

pub async fn delete_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<Deleted>, TenantError> {
    if !Scope::from(user).is_admin() {
        return Err(forbidden("Delete your prices by id"));
    }
    if !state.db.delete_model_pricing(&model_name).await.map_err(internal)? {
        return Err(not_found("Model pricing not found"));
    }
    Ok(Json(Deleted { deleted: true, model_name: Some(model_name), id: None }))
}

pub async fn delete_pricing_by_id(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<Deleted>, TenantError> {
    let scope = Scope::from(user);
    let row = visible_row(&scope, &state, id).await?;
    state.db.delete_pricing_row(row.id).await.map_err(internal)?;
    Ok(Json(Deleted { deleted: true, model_name: None, id: Some(id) }))
}
