use crate::auth::admin::AuthenticatedUser;
use crate::db::{UsageReport, UsageScope};
use crate::db::{Filter, Summary};
use crate::state::AppState;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

type ApiError = (StatusCode, String);

const DEFAULT_HOURS: u32 = 24;
const MAX_HOURS: u32 = 24 * 90;

#[derive(Deserialize)]
pub struct UsageQuery {
    hours: Option<u32>,
}

impl UsageQuery {
    fn hours(&self) -> u32 {
        self.hours.unwrap_or(DEFAULT_HOURS).clamp(1, MAX_HOURS)
    }
}

fn internal(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn report(state: &AppState, scope: UsageScope, query: &UsageQuery) -> Result<Json<UsageReport>, ApiError> {
    state.db.usage_report(scope, query.hours()).await.map(Json).map_err(internal)
}

pub async fn key_usage(
    Path(id): Path<i64>,
    Query(query): Query<UsageQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<UsageReport>, ApiError> {
    report(&state, UsageScope::Key(id), &query).await
}

pub async fn user_usage(
    Path(id): Path<i64>,
    Query(query): Query<UsageQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<UsageReport>, ApiError> {
    report(&state, UsageScope::User(id), &query).await
}

pub async fn my_usage(
    Extension(authenticated): Extension<AuthenticatedUser>,
    Query(query): Query<UsageQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<UsageReport>, ApiError> {
    let scope = match authenticated.api_key.as_ref() {
        Some(key) => UsageScope::Key(key.id),
        None => UsageScope::User(authenticated.user.id),
    };
    report(&state, scope, &query).await
}

#[derive(Deserialize)]
pub struct MyRequestsQuery {
    limit: Option<i64>,
    before: Option<i64>,
    hours: Option<u32>,
}

pub async fn my_requests(
    Extension(authenticated): Extension<AuthenticatedUser>,
    Query(query): Query<MyRequestsQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<Summary>>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let since_ms = query.hours.map(|h| {
        let now = chrono::Utc::now().timestamp_millis();
        now - i64::from(h.clamp(1, MAX_HOURS)) * 3_600_000
    });
    let filter = Filter {
        before: query.before,
        user_id: Some(authenticated.user.id),
        api_key_id: authenticated.api_key.as_ref().map(|k| k.id),
        since_ms,
        ..Default::default()
    };
    state.db.list_request_logs(limit, filter).await.map(Json).map_err(internal)
}
