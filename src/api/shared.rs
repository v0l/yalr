use axum::http::StatusCode;
use serde::Serialize;

use crate::auth::admin::AuthenticatedUser;
use crate::db::ModelAccess;
use crate::router::RouterError;
use crate::state::AppState;
use crate::ProviderError;

// ── Shared types used across API handlers ────────────────────────────

pub fn parse_extra_body(
    value: Option<serde_json::Value>,
) -> Result<Option<crate::db::ExtraBody>, (StatusCode, String)> {
    match value {
        None => Ok(None),
        Some(value) => crate::db::validate_extra_body(value).map_err(|e| (StatusCode::BAD_REQUEST, e)),
    }
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_available_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
}

impl ApiError {
    pub fn request_too_large(limit_bytes: usize) -> Self {
        Self {
            error: ApiErrorDetail {
                message: format!("request body is over the {} MiB limit", limit_bytes / (1024 * 1024)),
                error_type: "invalid_request_error".into(),
                code: Some("request_too_large".into()),
                limit_bytes: Some(limit_bytes),
                next_available_at: None,
                retry_after_seconds: None,
            },
        }
    }

    pub fn new(message: impl Into<String>, error_type: impl Into<String>) -> Self {
        Self {
            error: ApiErrorDetail {
                message: message.into(),
                error_type: error_type.into(),
                code: None,
                limit_bytes: None,
                next_available_at: None,
                retry_after_seconds: None,
            },
        }
    }

    pub fn insufficient_funds(required: i64, available: i64) -> Self {
        Self {
            error: ApiErrorDetail {
                message: "Insufficient funds".into(),
                error_type: "payment_required".into(),
                code: None,
                limit_bytes: None,
                next_available_at: None,
                retry_after_seconds: None,
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
    let access = state
        .db
        .effective_model_access(user_id, model, state.payments_state.is_some())
        .await
        .map_err(|e| {
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

#[derive(serde::Deserialize)]
struct UpstreamError {
    error: UpstreamErrorDetail,
}

#[derive(serde::Deserialize)]
struct UpstreamErrorDetail {
    message: String,
}

fn upstream_message(body: &str) -> String {
    serde_json::from_str::<UpstreamError>(body)
        .map(|e| e.error.message)
        .unwrap_or_else(|_| body.to_string())
}

pub const OUTAGE_RETRY_SECS: u64 = 30;

pub fn failure_response(e: &RouterError, cap_ms: Option<u64>) -> Option<HandlerError> {
    if e.is_outage() {
        let body = ApiError {
            error: ApiErrorDetail {
                message: format!("Upstream provider outage, retry shortly: {e}"),
                error_type: "upstream_unavailable".into(),
                code: Some("provider_outage".into()),
                limit_bytes: None,
                next_available_at: None,
                retry_after_seconds: Some(OUTAGE_RETRY_SECS),
            },
        };
        return Some((StatusCode::SERVICE_UNAVAILABLE, serde_json::to_string(&body).unwrap_or_default()));
    }
    let mut retry_ms = e.rate_limit_retry_ms()?;
    if let Some(cap) = cap_ms {
        retry_ms = retry_ms.min(cap);
    }
    let next_available_at = chrono::Utc::now().timestamp_millis() + retry_ms as i64;
    let body = ApiError {
        error: ApiErrorDetail {
            message: format!("Rate limit exceeded. Retry after: {retry_ms}ms"),
            error_type: "rate_limit_error".into(),
            code: Some("rate_limit_exceeded".into()),
            limit_bytes: None,
            next_available_at: Some(next_available_at),
            retry_after_seconds: Some(retry_ms.div_ceil(1000)),
        },
    };
    Some((StatusCode::TOO_MANY_REQUESTS, serde_json::to_string(&body).unwrap_or_default()))
}

pub fn router_error(e: RouterError, capability: &str) -> HandlerError {
    if let Some(limited) = failure_response(&e, None) {
        return limited;
    }
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
        RouterError::ProviderError(
            error @ ProviderError::ServerError {
                message,
                status_code: Some(code),
            },
        ) if error.is_client_error() => (
            StatusCode::from_u16(*code).unwrap_or(StatusCode::BAD_REQUEST),
            error_body(&upstream_message(message), "invalid_request_error"),
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            error_body(&e.to_string(), "router_error"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_error_envelope_is_unwrapped() {
        assert_eq!(
            upstream_message(r#"{"error":{"message":"Model x does not exist","code":400}}"#),
            "Model x does not exist"
        );
        assert_eq!(upstream_message("plain text"), "plain text");
    }
}
