use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::my_providers::{owned_provider, MyProvider};
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
pub struct MyProviderStats {
    pub provider: MyProvider,
    pub days: i64,
    pub traffic: ProviderTraffic,
    pub models: Vec<ProviderModelTraffic>,
    pub earnings: Vec<ProviderEarning>,
}

fn internal(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn stats(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(slug): Path<String>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<MyProviderStats>, (StatusCode, String)> {
    let provider = owned_provider(&state, &auth.user, &slug).await?;
    let days = query.days.unwrap_or(DEFAULT_DAYS).clamp(1, MAX_DAYS);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let since_ms = now_ms - days * DAY_MS;
    let traffic = state.db.provider_traffic(&provider.name, since_ms).await.map_err(internal)?;
    let models = state.db.provider_model_traffic(&provider.name, since_ms).await.map_err(internal)?;
    let earnings = match provider.owner_user_id {
        Some(owner) => state.db.provider_earnings(owner, &provider.name, since_ms).await.map_err(internal)?,
        None => Vec::new(),
    };
    Ok(Json(MyProviderStats { provider: provider.into(), days, traffic, models, earnings }))
}
