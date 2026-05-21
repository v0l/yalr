use crate::state::AppState;
use axum::{
    extract::{Extension, State},
    Json,
};
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};

/// Create a response using the Responses API
/// This endpoint uses the router's provider selection and retry logic
#[axum::debug_handler]
pub async fn create_response(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(user): Extension<Option<crate::db::User>>,
    Json(request): Json<CreateResponse>,
) -> Result<Json<ApiResponse>, (axum::http::StatusCode, String)> {
    tracing::info!(
        model = request.model.as_deref().unwrap_or("unknown"),
        stream = false,
        "Received Responses API request"
    );

    // ── Billing ──────────────────────────────────────────────
    let billing_guard = if state.payments_state.is_some() {
        let user_id = user.as_ref().map(|u| u.id);
        let model_name = request.model.as_deref().unwrap_or("unknown");
        match crate::payments::guard::BillingGuard::try_create(
            &state,
            user_id,
            model_name,
            None,
        )
        .await
        {
            Ok(guard) => Some(guard),
            Err(crate::payments::biller::BillingError::InsufficientFunds {
                required,
                available,
            }) => {
                let (code, json) =
                    crate::payments::guard::insufficient_funds_response(required, available);
                return Err((code, json.0.to_string()));
            }
            Err(e) => {
                tracing::error!(error = %e, "Billing reservation error");
                return Err((
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    serde_json::json!({"error": {"message": format!("Billing error: {}", e), "type": "billing_error"}}).to_string(),
                ));
            }
        }
    } else {
        None
    };
    // ──────────────────────────────────────────────────────────

    let metrics_user = user.as_ref().map(|u| crate::metrics::MetricsUser {
        id: Some(u.id),
        name: u.username.clone(),
        api_key_id: None,
        api_key_name: None,
    });

    match state.config.router.responses(&request, metrics_user).await {
        Ok(response) => {
            tracing::info!(
                model = request.model.as_deref().unwrap_or("unknown"),
                response_id = response.id,
                "Response created successfully"
            );

            // Finalize billing
            if let Some(guard) = &billing_guard {
                if let Some(ref usage) = response.usage {
                    guard.finalize(usage.input_tokens as u32, usage.output_tokens as u32).await;
                }
            }

            Ok(Json(response))
        },
        Err(e) => {
            tracing::error!(
                model = request.model.as_deref().unwrap_or("unknown"),
                error = %e,
                "Response creation failed"
            );
            let body = serde_json::json!({
                "error": {
                    "message": e.to_string(),
                    "type": "router_error",
                }
            });
            Err((axum::http::StatusCode::BAD_REQUEST, body.to_string()))
        }
    }
}
