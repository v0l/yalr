use crate::state::AppState;
use crate::ChatCompletionRequest;
use axum::{
    extract::{Extension, State},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use futures::Stream;
// serde is used transitively but not directly in this module

use std::convert::Infallible;
use std::sync::Arc;

#[axum::debug_handler]
pub async fn chat_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    if request.stream.unwrap_or(false) {
        let stream_response = chat_completions_stream(State(state), Extension(authenticated_user), Json(request)).await;
        Ok(stream_response.into_response())
    } else {
        let response = chat_completions_handler(State(state), Extension(authenticated_user), Json(request)).await?;
        Ok(response.into_response())
    }
}

#[axum::debug_handler]
pub async fn chat_completions_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<crate::ChatCompletionResponse>, (axum::http::StatusCode, String)> {
    let user = &authenticated_user.user;

    tracing::info!(
        model = request.model,
        stream = false,
        messages_count = request.messages.len(),
        "Received chat completion request"
    );

    // ── Billing ──────────────────────────────────────────────
    let billing_guard = if state.payments_state.is_some() {
        let user_id = Some(user.id);
        match crate::payments::guard::BillingGuard::try_create(
            &state,
            user_id,
            &request.model,
            request.max_completion_tokens.map(|t| t),
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

    // ── Model Access Control ───────────────────────────────────────
    // Check if user has access to this model
    let access_check = state.db.check_model_access(user.id, &request.model).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to check model access");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to check model access: {}", e))
        })?;

    // If there's an explicit permission rule, enforce it
    // None means no rule exists, so we allow by default
    if let Some(allowed) = access_check {
        if !allowed {
            return Err((
                axum::http::StatusCode::FORBIDDEN,
                serde_json::json!({
                    "error": {
                        "message": format!("User does not have access to model '{}'", request.model),
                        "type": "model_access_denied",
                    }
                })
                .to_string(),
            ));
        }
    }
    // ──────────────────────────────────────────────────────────

    let metrics_user = crate::metrics::MetricsUser {
        id: Some(user.id),
        name: user.username.clone(),
        api_key_id: authenticated_user.api_key.as_ref().map(|k| k.id),
        api_key_name: authenticated_user.api_key.as_ref().map(|k| k.name.clone()),
    };

    match state.config.router.chat_completions(&request, Some(metrics_user)).await {
        Ok(response) => {
            tracing::info!(
                model = request.model,
                completion_id = response.id,
                "Request completed successfully"
            );

            // Finalize billing
            if let Some(guard) = &billing_guard {
                if let Some(ref usage) = response.usage {
                    guard.finalize(usage.prompt_tokens, usage.completion_tokens).await;
                }
            }

            Ok(Json(response))
        },
        Err(e) => {
            tracing::error!(
                model = request.model,
                error = %e,
                "Routing failed"
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

#[axum::debug_handler]
pub async fn chat_completions_stream(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>, (axum::http::StatusCode, String)> {
    let user = &authenticated_user.user;

    tracing::info!(
        model = request.model,
        stream = true,
        messages_count = request.messages.len(),
        "Received streaming chat completion request"
    );

    // ── Billing ──────────────────────────────────────────────
    let billing_guard = if state.payments_state.is_some() {
        let user_id = Some(user.id);
        match crate::payments::guard::BillingGuard::try_create(
            &state,
            user_id,
            &request.model,
            request.max_completion_tokens,
        )
        .await
        {
            Ok(guard) => Some(Arc::new(guard)),
            Err(crate::payments::biller::BillingError::InsufficientFunds {
                required,
                available,
            }) => {
                return Err((
                    axum::http::StatusCode::PAYMENT_REQUIRED,
                    serde_json::json!({
                        "error": {
                            "message": "Insufficient funds",
                            "type": "payment_required",
                            "required_msat": required,
                            "available_msat": available,
                        }
                    })
                    .to_string(),
                ));
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

    // ── Model Access Control ───────────────────────────────────────
    // Check if user has access to this model
    let access_check = state.db.check_model_access(user.id, &request.model).await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to check model access");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to check model access: {}", e))
        })?;

    // If there's an explicit permission rule, enforce it
    // None means no rule exists, so we allow by default
    if let Some(allowed) = access_check {
        if !allowed {
            return Err((
                axum::http::StatusCode::FORBIDDEN,
                serde_json::json!({
                    "error": {
                        "message": format!("User does not have access to model '{}'", request.model),
                        "type": "model_access_denied",
                    }
                })
                .to_string(),
            ));
        }
    }
    // ──────────────────────────────────────────────────────────

    let metrics_user = crate::metrics::MetricsUser {
        id: Some(user.id),
        name: user.username.clone(),
        api_key_id: authenticated_user.api_key.as_ref().map(|k| k.id),
        api_key_name: authenticated_user.api_key.as_ref().map(|k| k.name.clone()),
    };

    let model = request.model.clone();
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send + 'static>> =
        match state.config.router.chat_completions_stream(&request, Some(metrics_user)).await {
            Ok(stream) => {
                let billing_guard = billing_guard.clone();
                let converted_stream = async_stream::stream! {
                    use futures::StreamExt;
                    let mut stream = stream;
                    let mut chunk_count = 0u32;
                    let mut prompt_tokens = 0u32;
                    let mut completion_tokens = 0u32;
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(chunk) => {
                                if let Some(ref usage) = chunk.usage {
                                    prompt_tokens = usage.prompt_tokens;
                                    completion_tokens = usage.completion_tokens;
                                }
                                chunk_count += 1;
                                yield Ok(Event::default().json_data(&chunk).unwrap_or_else(|_| Event::default()));
                            }
                            Err(e) => {
                                tracing::error!(
                                    model = %model,
                                    error = %e,
                                    chunks_sent = chunk_count,
                                    "Streaming request failed"
                                );
                                yield Ok(Event::default()
                                    .json_data(serde_json::json!({
                                        "error": {
                                            "message": e.to_string(),
                                            "type": "router_error",
                                        }
                                    }))
                                    .unwrap_or_else(|_| Event::default()));
                            }
                        }
                    }
                    if chunk_count > 0 {
                        tracing::info!(
                            model = %model,
                            chunks_sent = chunk_count,
                            prompt_tokens,
                            completion_tokens,
                            "Streaming request completed"
                        );
                    }

                    // Finalize billing after stream is done
                    if let Some(ref guard) = billing_guard {
                        if guard.is_active() {
                            guard.finalize(prompt_tokens, completion_tokens).await;
                        }
                    }
                };
                Box::pin(converted_stream)
            }
            Err(e) => {
                tracing::error!(
                    model = %model,
                    error = %e,
                    "Failed to create streaming route"
                );
                let error_stream = async_stream::stream! {
                    yield Ok(Event::default()
                        .json_data(serde_json::json!({
                            "error": {
                                "message": e.to_string(),
                                "type": "router_error",
                            }
                        }))
                        .unwrap_or_else(|_| Event::default()));
                };
                Box::pin(error_stream)
            }
        };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new()))
}
