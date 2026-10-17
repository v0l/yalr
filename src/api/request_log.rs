use crate::request_log::{codec::unpack, store};
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

type ApiError = (StatusCode, String);

fn internal(e: impl std::fmt::Display) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
    before: Option<i64>,
    model: Option<String>,
    status: Option<String>,
}

pub async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<store::Summary>>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    store::list(&state.db.pool, limit, query.before, query.model.as_deref(), query.status.as_deref())
        .await
        .map(Json)
        .map_err(internal)
}

#[derive(Serialize)]
pub struct DetailResponse {
    #[serde(flatten)]
    detail: store::Detail,
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

pub async fn detail(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<DetailResponse>, ApiError> {
    let detail = store::detail(&state.db.pool, id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "no such request".to_string()))?;
    let has_bodies = store::bodies(&state.db.pool, id).await.map_err(internal)?.is_some();
    let previous_log_id = match detail.previous_request_id.as_deref() {
        Some(previous) => store::id_for_request(&state.db.pool, previous).await.map_err(internal)?,
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
    Path(id): Path<i64>,
) -> Result<Json<BodiesResponse>, ApiError> {
    let bodies = store::bodies(&state.db.pool, id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "no bodies stored for this request (expired or disabled)".to_string()))?;
    Ok(Json(BodiesResponse {
        client_request: decode(bodies.client_request_packed),
        upstream_request: decode(bodies.upstream_request_packed),
        response: decode(bodies.response_packed),
    }))
}
