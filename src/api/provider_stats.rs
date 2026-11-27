use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::tenant::{internal, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{ProviderEarning, ProviderModelTraffic, ProviderTraffic};
use crate::state::AppState;

const DAY_MS: i64 = 86_400_000;
const DEFAULT_DAYS: i64 = 30;
const MAX_DAYS: i64 = 365;

#[derive(Deserialize)]
pub struct StatsQuery {
    pub days: Option<i64>,
}

#[derive(Serialize)]
pub struct ProviderStats {
    pub slug: String,
    pub name: String,
    pub base_url: String,
    pub owner_user_id: Option<i64>,
    pub days: i64,
    pub traffic: ProviderTraffic,
    pub models: Vec<ProviderModelTraffic>,
    pub earnings: Vec<ProviderEarning>,
}

#[derive(Serialize)]
pub struct ProviderCheck {
    pub healthy: bool,
    pub error: Option<String>,
}

pub async fn stats(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(slug): Path<String>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ProviderStats>, TenantError> {
    let provider = Scope::from(user).provider(&state, &slug).await?;
    let days = query.days.unwrap_or(DEFAULT_DAYS).clamp(1, MAX_DAYS);
    let since_ms = chrono::Utc::now().timestamp_millis() - days * DAY_MS;
    let traffic = state.db.provider_traffic(&provider.name, since_ms).await.map_err(internal)?;
    let models = state.db.provider_model_traffic(&provider.name, since_ms).await.map_err(internal)?;
    let earnings = match provider.owner_user_id {
        Some(owner) => state.db.provider_earnings(owner, &provider.name, since_ms).await.map_err(internal)?,
        None => Vec::new(),
    };
    Ok(Json(ProviderStats {
        slug: provider.slug,
        name: provider.name,
        base_url: provider.base_url,
        owner_user_id: provider.owner_user_id,
        days,
        traffic,
        models,
        earnings,
    }))
}

pub async fn check(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(slug): Path<String>,
) -> Result<Json<ProviderCheck>, TenantError> {
    let scope = Scope::from(user);
    let record = scope.provider(&state, &slug).await?;
    let live = state
        .config
        .router
        .get_providers()
        .await
        .into_iter()
        .find(|p| p.slug() == slug)
        .ok_or((StatusCode::BAD_GATEWAY, "Provider is not loaded".to_string()))?;
    let result = crate::api::abuse::owner_contact(&state, &scope, &record, live.health_check()).await?;
    Ok(Json(ProviderCheck { healthy: matches!(result, Ok(true)), error: result.err().map(|e| e.to_string()) }))
}
