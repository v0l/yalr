use crate::state::AppState;
use crate::db::ModelAccess;
use crate::providers::ChatRequest;
use axum::{
    extract::{ConnectInfo, Extension, State},
    http::HeaderMap,
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use crate::request_log::{recorder::{Inbound, Pending}, redact, settings, Trace};
use std::net::SocketAddr;
use futures::Stream;
// serde is used transitively but not directly in this module

use std::convert::Infallible;
use std::sync::Arc;

fn reject(log: Option<Pending>, status: &str, reason: &str) {
    if let Some(pending) = log {
        pending.complete(status, Some(reason.to_string()));
    }
}

fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string());
    forwarded.or_else(|| peer.map(|p| p.ip().to_string()))
}

fn begin_log(
    state: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    user: &crate::auth::admin::AuthenticatedUser,
    request: &ChatRequest,
    raw_body: &[u8],
) -> Option<Pending> {
    let settings = settings::get();
    if !settings.enabled {
        return None;
    }
    let inbound = Inbound {
        request_id: uuid::Uuid::new_v4().to_string(),
        endpoint: "/v1/chat/completions",
        stream: request.stream.unwrap_or(false),
        model: request.model.clone(),
        user_id: Some(user.user.id),
        user_name: user.user.username.clone(),
        api_key_id: user.api_key.as_ref().map(|k| k.id),
        api_key_name: user.api_key.as_ref().map(|k| k.name.clone()),
        client_ip: client_ip(headers, peer),
        client_headers: redact::axum_headers(headers),
        body: if settings.store_bodies { raw_body.to_vec() } else { Vec::new() },
    };
    Some(Pending::new(state.db.pool.clone(), inbound, Trace::new()))
}

#[axum::debug_handler]
pub async fn chat_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    let is_json = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
        .is_some_and(|v| v == "application/json" || (v.starts_with("application/") && v.ends_with("+json")));
    if !is_json {
        return Err((
            axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Expected request with `Content-Type: application/json`".to_string(),
        ));
    }
    let request: ChatRequest = serde_json::from_slice(&body).map_err(|e| {
        let syntax = serde_json::from_slice::<serde_json::Value>(&body).is_err();
        if syntax {
            (axum::http::StatusCode::BAD_REQUEST, format!("Failed to parse the request body as JSON: {e}"))
        } else {
            (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                format!("Failed to deserialize the JSON body into the target type: {e}"),
            )
        }
    })?;
    let (input, output) = crate::api::modalities::chat_modalities(&request);
    crate::api::modalities::require_modalities(&state, &request.model, &input, &output).await?;

    let log = begin_log(&state, &headers, peer.map(|p| p.0), &authenticated_user, &request, &body);

    if request.stream.unwrap_or(false) {
        let stream_response = chat_completions_stream(State(state), Extension(authenticated_user), request, log).await;
        Ok(stream_response.into_response())
    } else {
        let response = chat_completions_handler(State(state), Extension(authenticated_user), request, log).await?;
        Ok(response.into_response())
    }
}

// ── Model Access Control ───────────────────────────────────────

async fn check_model_access(
    state: &AppState,
    user_id: i64,
    model: &str,
) -> Result<(), (axum::http::StatusCode, String)> {
    crate::api::shared::check_model_access(state, user_id, model)
        .await
        .map_err(|(code, body)| (code, body.to_string()))
}

// ──────────────────────────────────────────────────────────

pub async fn chat_completions_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    request: ChatRequest,
    mut log: Option<Pending>,
) -> Result<Json<crate::ChatCompletionResponse>, (axum::http::StatusCode, String)> {
    let user = &authenticated_user.user;
    let request = match &log {
        Some(pending) => request.with_trace(pending.trace().clone()),
        None => request,
    };

    tracing::info!(
        model = request.model,
        stream = false,
        messages_count = request.messages.len(),
        "Received chat completion request"
    );

    // ── Model Access Control ───────────────────────────────────────
    if let Err(denied) = check_model_access(&state, user.id, &request.model).await {
        reject(log.take(), "rejected", &denied.1);
        return Err(denied);
    }
    // ──────────────────────────────────────────────────────────

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
                reject(log.take(), "rejected", "insufficient funds");
                return Err((code, json.0.to_string()));
            }
            Err(e) => {
                tracing::error!(error = %e, "Billing reservation error");
                reject(log.take(), "error", &format!("billing error: {e}"));
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

    let metrics_user = crate::metrics::MetricsUser {
        id: Some(user.id),
        name: user.username.clone(),
        api_key_id: authenticated_user.api_key.as_ref().map(|k| k.id),
        api_key_name: authenticated_user.api_key.as_ref().map(|k| k.name.clone()),
    };

    match state.config.router.chat_completions(&request, Some(metrics_user)).await {
        Ok(response) => {
            if let Some(mut pending) = log.take() {
                if let Some(usage) = response.usage.as_ref() {
                    pending.observe_usage(usage, None);
                }
                let finish = response
                    .choices
                    .first()
                    .and_then(|c| c.finish_reason.as_ref())
                    .and_then(|f| serde_json::to_value(f).ok())
                    .and_then(|v| v.as_str().map(str::to_string));
                pending.set_response(serde_json::to_vec(&response).unwrap_or_default(), finish);
                pending.complete("ok", None);
            }
            let cached_tokens = response
                .usage
                .as_ref()
                .and_then(|u| u.prompt_tokens_details.as_ref())
                .and_then(|d| d.cached_tokens)
                .unwrap_or(0);
            tracing::info!(
                model = request.model,
                completion_id = response.id,
                cached_tokens,
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
            if let Some(guard) = &billing_guard {
                guard.release().await;
            }
            if let Some(pending) = log.take() {
                pending.complete("error", Some(e.to_string()));
            }
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

pub async fn chat_completions_stream(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
    request: ChatRequest,
    mut log: Option<Pending>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>, (axum::http::StatusCode, String)> {
    let user = &authenticated_user.user;
    let request = match &log {
        Some(pending) => request.with_trace(pending.trace().clone()),
        None => request,
    };

    tracing::info!(
        model = request.model,
        stream = true,
        messages_count = request.messages.len(),
        "Received streaming chat completion request"
    );

    // ── Model Access Control ───────────────────────────────────────
    if let Err(denied) = check_model_access(&state, user.id, &request.model).await {
        reject(log.take(), "rejected", &denied.1);
        return Err(denied);
    }
    // ──────────────────────────────────────────────────────────

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
                reject(log.take(), "rejected", "insufficient funds");
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
                reject(log.take(), "error", &format!("billing error: {e}"));
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
                let mut log = log;
                let converted_stream = async_stream::stream! {
                    use futures::StreamExt;
                    let mut stream = stream;
                    let mut stream_error: Option<String> = None;
                    let mut chunk_count = 0u32;
                    let mut prompt_tokens = 0u32;
                    let mut completion_tokens = 0u32;
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(chunk) => {
                                if let Some(pending) = log.as_mut() {
                                    pending.observe(&chunk);
                                }
                                if let Some(ref usage) = chunk.usage {
                                    prompt_tokens = usage.prompt_tokens;
                                    completion_tokens = usage.completion_tokens;
                                }
                                chunk_count += 1;
                                yield Ok(Event::default().json_data(&chunk).unwrap_or_else(|_| Event::default()));
                            }
                            Err(e) => {
                                stream_error = Some(e.to_string());
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
                    if let Some(pending) = log.take() {
                        match stream_error.take() {
                            Some(error) => pending.complete("error", Some(error)),
                            None => pending.complete("ok", None),
                        }
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
                if let Some(guard) = &billing_guard {
                    guard.release().await;
                }
                if let Some(pending) = log {
                    pending.complete("error", Some(e.to_string()));
                }
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
