use std::sync::Arc;

use axum::{extract::State, Json};

use crate::api::tenant::{internal, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{FailoverPenaltyRow, FailoverPenaltyTotal};
use crate::state::AppState;

const RECENT: i64 = 200;
const DAYS: i64 = 30;
const DAY_MS: i64 = 86_400_000;

pub async fn mine(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
) -> Result<Json<Vec<FailoverPenaltyRow>>, TenantError> {
    let rows = state.db.failover_penalties_for_owner(auth.user.id, RECENT).await.map_err(internal)?;
    Ok(Json(rows))
}

pub async fn totals(State(state): State<Arc<AppState>>) -> Result<Json<Vec<FailoverPenaltyTotal>>, TenantError> {
    let since = chrono::Utc::now().timestamp_millis() - DAYS * DAY_MS;
    let rows = state.db.failover_penalty_totals(since).await.map_err(internal)?;
    Ok(Json(rows))
}
