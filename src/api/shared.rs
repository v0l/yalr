use serde::Serialize;

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
