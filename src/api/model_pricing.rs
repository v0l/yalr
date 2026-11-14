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

#[axum::debug_handler]
pub async fn list_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<ModelPricingResponse>> {
    let rows = state.config.db.list_model_pricings().await.unwrap_or_default();
    let mut out = Vec::new();
    for row in rows {
        out.push(response(&state, row).await);
    }
    Json(out)
}

#[axum::debug_handler]
pub async fn create_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<ModelPricingCreateRequest>,
) -> Result<Json<ModelPricingResponse>, (StatusCode, String)> {
    let mp = crate::db::NewModelPricing {
        model_name: req.model_name.trim(),
        is_advertised: req.is_advertised,
        is_free: req.is_free,
        price_per_1m_input_sats: req.price_per_1m_input_sats,
        price_per_1m_output_sats: req.price_per_1m_output_sats,
        price_per_1m_cache_read_sats: req.price_per_1m_cache_read_sats,
        price_per_1m_cache_write_sats: req.price_per_1m_cache_write_sats,
        price_per_request_sats: req.price_per_request_sats,
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
        owner_user_id: None,
        provider_id: None,
        currency: "sats",
        fiat: Default::default(),
    };

    match state.config.db.create_model_pricing(mp).await {
        Ok(r) => Ok(Json(response(&state, r).await)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[axum::debug_handler]
pub async fn update_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<ModelPricingUpdateRequest>,
) -> Result<Json<ModelPricingResponse>, (StatusCode, String)> {
    let updates = crate::db::UpdateModelPricing {
        is_advertised: req.is_advertised,
        is_free: req.is_free,
        price_per_1m_input_sats: req.price_per_1m_input_sats,
        price_per_1m_output_sats: req.price_per_1m_output_sats,
        price_per_1m_cache_read_sats: req.price_per_1m_cache_read_sats,
        price_per_1m_cache_write_sats: req.price_per_1m_cache_write_sats,
        price_per_request_sats: req.price_per_request_sats,
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
        currency: None,
        fiat: None,
    };

    match state.config.db.update_model_pricing(&model_name, updates).await {
        Ok(r) => Ok(Json(response(&state, r).await)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[axum::debug_handler]
pub async fn delete_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.config.db.delete_model_pricing(&model_name).await {
        Ok(true) => Ok(Json(serde_json::json!({"deleted": true, "model_name": model_name}))),
        Ok(false) => Err((StatusCode::NOT_FOUND, "Model pricing not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
