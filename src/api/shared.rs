use axum::http::StatusCode;
use serde::Serialize;

use crate::auth::admin::AuthenticatedUser;
use crate::db::ModelAccess;
use crate::router::RouterError;
use crate::state::AppState;
use crate::ProviderError;

// ── Shared types used across API handlers ────────────────────────────

/// API error response body.
#[derive(Serialize)]
pub struct ApiError {
    pub error: ApiErrorDetail,
}

#[derive(Serialize)]
pub struct ApiErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ApiError {
    pub fn new(message: impl Into<String>, error_type: impl Into<String>) -> Self {
        Self {
            error: ApiErrorDetail {
                message: message.into(),
                error_type: error_type.into(),
                code: None,
            },
        }
    }

    pub fn insufficient_funds(required: i64, available: i64) -> Self {
        Self {
            error: ApiErrorDetail {
                message: "Insufficient funds".into(),
                error_type: "payment_required".into(),
                code: None,
            },
        }
    }
}

/// Generic success/fail message returned by CRUD mutation endpoints.
#[derive(Serialize)]
pub struct MutationResponse {
    pub success: bool,
    pub message: String,
}

// ── Provider password struct ─────────────────────────────────────────────

#[derive(Serialize)]
pub struct ProviderPasswordResponse {
    pub success: bool,
    pub provider: String,
    pub api_key: String,
    pub masked_key: String,
}

pub type HandlerError = (StatusCode, String);

pub fn error_body(message: &str, kind: &str) -> String {
    serde_json::to_string(&ApiError::new(message, kind)).unwrap_or_default()
}

pub async fn check_model_access(
    state: &AppState,
    user_id: i64,
    model: &str,
) -> Result<(), HandlerError> {
    let access = state.db.check_model_access(user_id, model).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to check model access");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body(&format!("Failed to check model access: {e}"), "internal_error"),
        )
    })?;
    if access == ModelAccess::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            error_body(
                &format!("User does not have access to model '{model}'"),
                "model_access_denied",
            ),
        ));
    }
    Ok(())
}

pub fn metrics_user(authenticated: &AuthenticatedUser) -> crate::metrics::MetricsUser {
    crate::metrics::MetricsUser {
        id: Some(authenticated.user.id),
        name: authenticated.user.username.clone(),
        api_key_id: authenticated.api_key.as_ref().map(|k| k.id),
        api_key_name: authenticated.api_key.as_ref().map(|k| k.name.clone()),
    }
}

pub fn router_error(e: RouterError, capability: &str) -> HandlerError {
    match &e {
        RouterError::NoAvailableProvider => (
            StatusCode::NOT_FOUND,
            error_body(&e.to_string(), "router_error"),
        ),
        RouterError::ProviderError(ProviderError::Unsupported(_)) => (
            StatusCode::NOT_FOUND,
            error_body(
                &format!("No configured provider can serve {capability} for this model"),
                "model_not_supported",
            ),
        ),
        RouterError::ProviderError(ProviderError::ServerError {
            message,
            status_code: Some(code @ (400 | 413 | 422)),
        }) => (
            StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_REQUEST),
            error_body(message, "invalid_request_error"),
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            error_body(&e.to_string(), "router_error"),
        ),
    }
}
