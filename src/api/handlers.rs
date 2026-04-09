use crate::api::server::AppState;
use axum::{
    extract::{Path, State},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;

use crate::{ChatCompletionRequest, ChatCompletionResponse};
use crate::router::{DbModelInfo, ModelInfoDetector};

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub timestamp: u64,
}

#[derive(Serialize)]
pub struct ProviderMetrics {
    pub provider: String,
    pub p90_tokens_per_second: Option<f32>,
    pub p90_ttft_ms: Option<u32>,
    pub avg_latency_ms: Option<f32>,
    pub success_rate: Option<f32>,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub providers: Vec<ProviderMetrics>,
    pub recent_events: Vec<serde_json::Value>,
}

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    })
}

pub async fn get_metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    let providers = vec![
        {
            let summary = state.metrics_store.get_provider_summary("openai-primary").await;
            ProviderMetrics {
                provider: summary.provider,
                p90_tokens_per_second: summary.p90_output_tokens_per_second,
                p90_ttft_ms: summary.p90_ttft,
                avg_latency_ms: summary.avg_latency,
                success_rate: summary.success_rate,
            }
        },
        {
            let summary = state.metrics_store.get_provider_summary("openai-secondary").await;
            ProviderMetrics {
                provider: summary.provider,
                p90_tokens_per_second: summary.p90_output_tokens_per_second,
                p90_ttft_ms: summary.p90_ttft,
                avg_latency_ms: summary.avg_latency,
                success_rate: summary.success_rate,
            }
        },
    ];

    // Get model-specific summaries (available for future use)
    let _model_summaries = state.metrics_store.get_model_summaries_for_provider("openai-primary").await;

    let recent_events: Vec<serde_json::Value> = state
        .metrics_store
        .recent_events(50)
        .await
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();

    Json(MetricsResponse {
        providers,
        recent_events,
    })
}

pub async fn list_models(State(_state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "object": "list",
        "data": []
    }))
}

pub async fn chat_handler(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    if request.stream.unwrap_or(false) {
        let stream_response = chat_completions_stream(State(state), Json(request)).await;
        Ok(stream_response.into_response())
    } else {
        let response = chat_completions_handler(State(state), Json(request)).await?;
        Ok(response.into_response())
    }
}

#[axum::debug_handler]
pub async fn chat_completions_handler(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, (axum::http::StatusCode, String)> {
    tracing::info!(
        model = request.model,
        stream = false,
        messages_count = request.messages.len(),
        "Received chat completion request"
    );

    match state.config.router.chat_completions(&request).await {
        Ok(response) => {
            tracing::info!(
                model = request.model,
                completion_id = response.id,
                "Request completed successfully"
            );
            Ok(Json(response))
        },
        Err(e) => {
            tracing::error!(
                model = request.model,
                error = %e,
                "Routing failed"
            );
            Ok(Json(ChatCompletionResponse {
                id: "error".to_string(),
                object: "error".to_string(),
                created: 0,
                model: request.model,
                choices: vec![],
                usage: None,
                service_tier: None,
                #[allow(deprecated)]
                system_fingerprint: None,
            }))
        }
    }
}

#[axum::debug_handler]
pub async fn chat_completions_stream(
    State(state): State<AppState>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>, (axum::http::StatusCode, String)> {
    tracing::info!(
        model = request.model,
        stream = true,
        messages_count = request.messages.len(),
        "Received streaming chat completion request"
    );

    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send + 'static>> = 
        match state.config.router.chat_completions_stream(&request).await {
            Ok(stream) => {
                let converted_stream = async_stream::stream! {
                    use futures::StreamExt;
                    let mut stream = stream;
                    let mut chunk_count = 0;
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(chunk) => {
                                chunk_count += 1;
                                yield Ok(Event::default().json_data(&chunk).unwrap_or_else(|_| Event::default()));
                            }
                            Err(e) => {
                                tracing::error!(
                                    model = request.model,
                                    error = %e,
                                    chunks_sent = chunk_count,
                                    "Streaming request failed"
                                );
                                yield Ok(Event::default()
                                    .json_data(serde_json::json!({ "error": e.to_string() }))
                                    .unwrap_or_else(|_| Event::default()));
                            }
                        }
                    }
                    if chunk_count > 0 {
                        tracing::info!(
                            model = request.model,
                            chunks_sent = chunk_count,
                            "Streaming request completed"
                        );
                    }
                };
                Box::pin(converted_stream)
            }
            Err(e) => {
                tracing::error!(
                    model = request.model,
                    error = %e,
                    "Failed to create streaming route"
                );
                let error_stream = async_stream::stream! {
                    yield Ok(Event::default()
                        .json_data(serde_json::json!({ "error": e.to_string() }))
                        .unwrap_or_else(|_| Event::default()));
                };
                Box::pin(error_stream)
            }
        };

    Ok(Sse::new(stream).keep_alive(KeepAlive::new()))
}

#[derive(Deserialize)]
pub struct ProviderCreateRequest {
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: String,
}

#[axum::debug_handler]
pub async fn list_providers(State(_state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "providers": []
    }))
}

#[axum::debug_handler]
pub async fn create_provider(
    State(state): State<AppState>,
    Json(request): Json<ProviderCreateRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    use crate::providers::openai::OpenAiProvider;
    use std::sync::Arc;

    let provider = Arc::new(OpenAiProvider::new(
        &request.name,
        Some(&request.slug),
        &request.base_url,
        Some(&request.api_key),
    ));

    sqlx::query(
        "INSERT INTO providers (name, slug, base_url, api_key) VALUES (?, ?, ?, ?)",
    )
    .bind(&request.name)
    .bind(&request.slug)
    .bind(&request.base_url)
    .bind(&request.api_key)
    .execute(&state.config.db.pool)
    .await
    .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        provider_name = request.name,
        provider_slug = request.slug,
        base_url = request.base_url,
        "Adding provider to router"
    );
    
    state.config.router.add_provider(provider).await;
    
    tracing::info!(
        provider_name = request.name,
        "Provider added successfully"
    );

    Ok(Json(serde_json::json!({
        "id": 0,
        "name": request.name,
        "slug": request.slug,
        "base_url": request.base_url,
        "created_at": "now"
    })))
}

#[axum::debug_handler]
pub async fn delete_provider(
    Path(slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    tracing::info!(provider_slug = slug, "Deleting provider");
    
    sqlx::query("DELETE FROM providers WHERE slug = ?")
        .bind(&slug)
        .execute(&state.config.db.pool)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Provider deleted successfully");

    Ok(Json(serde_json::json!({
        "deleted": true,
        "slug": slug
    })))
}

#[derive(Serialize)]
pub struct ModelSyncReportResponse {
    pub model_name: String,
    pub provider_name: String,
    pub discrepancies: Vec<ModelDiscrepancyResponse>,
    pub is_synced: bool,
}

#[derive(Serialize)]
pub struct ModelDiscrepancyResponse {
    pub field: String,
    pub database_value: Option<String>,
    pub api_value: Option<String>,
    pub severity: String,
}

#[derive(Deserialize)]
pub struct ModelSyncRequest {
    pub models: HashMap<String, DbModelInfo>,
}

#[axum::debug_handler]
pub async fn detect_model_discrepancies(
    State(state): State<AppState>,
    Json(request): Json<ModelSyncRequest>,
) -> Json<Vec<ModelSyncReportResponse>> {
    let providers = state.config.router.get_providers().await;
    let detector = ModelInfoDetector::new(providers);

    let reports = detector.detect_discrepancies(&request.models).await;

    let response: Vec<ModelSyncReportResponse> = reports
        .into_iter()
        .map(|report| {
            let discrepancies = report.discrepancies
                .into_iter()
                .map(|d| ModelDiscrepancyResponse {
                    field: d.field,
                    database_value: d.database_value,
                    api_value: d.api_value,
                    severity: match d.severity {
                        crate::router::DiscrepancySeverity::Info => "info".to_string(),
                        crate::router::DiscrepancySeverity::Warning => "warning".to_string(),
                        crate::router::DiscrepancySeverity::Error => "error".to_string(),
                    },
                })
                .collect();

            ModelSyncReportResponse {
                model_name: report.model_name,
                provider_name: report.provider_name,
                discrepancies,
                is_synced: report.is_synced,
            }
        })
        .collect();

    Json(response)
}

#[axum::debug_handler]
pub async fn sync_provider_models(
    Path(provider_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let providers = state.config.router.get_providers().await;
    let provider = providers
        .iter()
        .find(|p| p.slug() == provider_slug)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", provider_slug)))?;

    match provider.list_models().await {
        Ok(models) => {
            let mut model_details = Vec::new();
            
            for model in &models {
                match provider.get_runtime_info(&model.id).await {
                    Ok(Some(info)) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "object": model.object,
                            "created": model.created,
                            "owned_by": model.owned_by,
                            "context_length": info.context_length(),
                            "quantization": info.quantization(),
                            "parameter_size": info.parameter_size(),
                            "max_output_tokens": info.max_output_tokens,
                            "additional_fields": info.additional_fields,
                        }));
                    }
                    Ok(None) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "object": model.object,
                            "created": model.created,
                            "owned_by": model.owned_by,
                            "runtime_info": null,
                        }));
                    }
                    Err(e) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "error": e.to_string(),
                        }));
                    }
                }
            }

            Ok(Json(serde_json::json!({
                "provider": provider_slug,
                "models": model_details,
                "total_count": models.len(),
            })))
        }
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
