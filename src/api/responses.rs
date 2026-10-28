use std::net::SocketAddr;
use std::sync::Arc;

use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use axum::{
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use bytes::Bytes;

use crate::api::metered::{Metered, RequestInfo};
use crate::api::shared::{check_model_access, error_body, metrics_user, HandlerError};
use crate::auth::admin::AuthenticatedUser;
use crate::payments::biller::TokenUsage;
use crate::state::AppState;

fn billed(response: &ApiResponse) -> Option<TokenUsage> {
    response.usage.as_ref().map(|usage| TokenUsage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        cached_tokens: usage.input_tokens_details.cached_tokens,
        cache_write_tokens: 0,
    })
}

#[axum::debug_handler]
pub async fn create_response(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ApiResponse>, HandlerError> {
    let request: CreateResponse = serde_json::from_slice(&body).map_err(|e| {
        (StatusCode::UNPROCESSABLE_ENTITY, error_body(&format!("Invalid request body: {e}"), "invalid_request_error"))
    })?;
    let model = request.model.clone().unwrap_or_default();
    let info = RequestInfo {
        endpoint: "/v1/responses",
        model: &model,
        stream: request.stream.unwrap_or(false),
        headers: &headers,
        peer: peer.map(|p| p.0),
    };
    let mut metered = Metered::begin(&state, &authenticated, info, || body.to_vec());

    tracing::info!(model = %model, "Received Responses API request");

    if let Err(e) = check_model_access(&state, authenticated.user.id, &model).await {
        return Err(metered.reject(e));
    }
    if let Err(e) = metered.admit().await {
        return Err(metered.reject(e));
    }

    let session = crate::api::chat::session_header(&headers);
    let result = state
        .config
        .router
        .responses(&request, session.as_deref(), Some(metrics_user(&authenticated)), Some(metered.trace()))
        .await;

    match result {
        Ok(response) => {
            tracing::info!(model = %model, response_id = response.id, "Response created successfully");
            let logged = serde_json::to_vec(&response).unwrap_or_default();
            metered.succeed(logged, billed(&response)).await;
            Ok(Json(response))
        }
        Err(e) => {
            tracing::error!(model = %model, error = %e, "Response creation failed");
            metered.fail(&e.to_string());
            Err((StatusCode::BAD_REQUEST, error_body(&e.to_string(), "router_error")))
        }
    }
}
