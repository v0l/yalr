use crate::api::tenant::{Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{Detail, Filter, Summary};
use crate::request_log::codec::unpack;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type ApiError = TenantError;

fn internal(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
    before: Option<i64>,
    model: Option<String>,
    status: Option<String>,
    user_id: Option<i64>,
    api_key_id: Option<i64>,
    endpoint: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Summary>>, ApiError> {
    let scope = Scope::from(user);
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let filter = Filter {
        before: query.before,
        model: query.model.as_deref(),
        status: query.status.as_deref(),
        user_id: scope.owner().or(query.user_id),
        api_key_id: query.api_key_id,
        endpoint: query.endpoint.as_deref(),
        since_ms: None,
    };
    let mut rows = state.db.list_request_logs(limit, filter).await.map_err(internal)?;
    if let Some(own) = scope.provider_names(&state).await? {
        for row in rows.iter_mut().filter(|r| !r.provider.as_ref().is_some_and(|p| own.contains(p))) {
            row.route = None;
        }
    }
    Ok(Json(rows))
}

#[derive(Serialize)]
pub struct DetailResponse {
    #[serde(flatten)]
    detail: Detail,
    client_headers_parsed: serde_json::Value,
    upstream_request_headers_parsed: serde_json::Value,
    upstream_response_headers_parsed: serde_json::Value,
    attempt_log_parsed: serde_json::Value,
    message_hashes_parsed: serde_json::Value,
    has_bodies: bool,
    previous_log_id: Option<i64>,
}

fn parse(text: &Option<String>) -> serde_json::Value {
    text.as_deref()
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or(serde_json::Value::Null)
}

struct Access {
    upstream: bool,
}

async fn visible(state: &AppState, scope: &Scope, id: i64) -> Result<(Detail, Access), ApiError> {
    let detail = state
        .db
        .request_log_detail(id)
        .await
        .map_err(internal)?
        .filter(|d| scope.is_admin() || d.user_id == Some(scope.user.id))
        .ok_or((StatusCode::NOT_FOUND, "no such request".to_string()))?;
    let upstream = scope.is_admin() || serves_own_provider(state, scope, detail.provider.as_deref()).await?;
    Ok((detail, Access { upstream }))
}

async fn serves_own_provider(state: &AppState, scope: &Scope, provider: Option<&str>) -> Result<bool, ApiError> {
    let Some(name) = provider else { return Ok(false) };
    Ok(scope.provider_names(state).await?.is_some_and(|names| names.iter().any(|n| n == name)))
}

fn hide_upstream(detail: &mut Detail) {
    detail.upstream_url = None;
    detail.upstream_request_headers = None;
    detail.upstream_response_headers = None;
    detail.attempt_log = None;
    detail.route = None;
}

pub async fn detail(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(id): Path<i64>,
) -> Result<Json<DetailResponse>, ApiError> {
    let scope = Scope::from(user);
    let (mut detail, access) = visible(&state, &scope, id).await?;
    if !access.upstream {
        hide_upstream(&mut detail);
    }
    let has_bodies = state.db.request_log_has_bodies(id).await.map_err(internal)?;
    let previous_log_id = match detail.previous_request_id.as_deref() {
        Some(previous) => state.db.request_log_id(previous).await.map_err(internal)?,
        None => None,
    };
    Ok(Json(DetailResponse {
        client_headers_parsed: parse(&detail.client_headers),
        upstream_request_headers_parsed: parse(&detail.upstream_request_headers),
        upstream_response_headers_parsed: parse(&detail.upstream_response_headers),
        attempt_log_parsed: parse(&detail.attempt_log),
        message_hashes_parsed: parse(&detail.message_hashes),
        has_bodies,
        previous_log_id,
        detail,
    }))
}

#[derive(Serialize)]
pub struct BodiesResponse {
    client_request: serde_json::Value,
    upstream_request: serde_json::Value,
    response: serde_json::Value,
}

fn decode(bytes: Option<Vec<u8>>) -> serde_json::Value {
    let Some(raw) = bytes.as_deref().and_then(unpack) else {
        return serde_json::Value::Null;
    };
    serde_json::from_slice(&raw)
        .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&raw).into_owned()))
}

pub async fn bodies(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(id): Path<i64>,
) -> Result<Json<BodiesResponse>, ApiError> {
    let scope = Scope::from(user);
    let (_, access) = visible(&state, &scope, id).await?;
    let bodies = state.db.request_log_bodies(id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "no bodies stored for this request (expired or disabled)".to_string()))?;
    Ok(Json(BodiesResponse {
        client_request: decode(bodies.client_request_packed),
        upstream_request: if access.upstream { decode(bodies.upstream_request_packed) } else { serde_json::Value::Null },
        response: decode(bodies.response_packed),
    }))
}
