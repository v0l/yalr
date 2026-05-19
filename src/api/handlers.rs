use crate::state::AppState;
use crate::db::UserType;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    Json,
};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use crate::{ChatCompletionRequest, ChatCompletionResponse};
use crate::router::{DbModelInfo, ModelInfoDetector};
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub timestamp: u64,
}

#[derive(Serialize)]
pub struct ProviderMetrics {
    pub provider: String,
    pub p90_tokens_per_second: Option<f32>,
    pub p90_input_tokens_per_second: Option<f32>,
    pub p90_ttft_ms: Option<u32>,
    pub avg_latency_ms: Option<f32>,
    pub success_rate: Option<f32>,
    pub health_state: Option<String>,
    pub consecutive_failures: Option<u32>,
    pub in_flight: Option<u32>,
    pub max_concurrency: Option<u32>,
    pub backoff_ms: Option<u64>,
    pub load_score: Option<f32>,
    pub available: Option<bool>,
    /// Full health entry with balance tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ProviderHealthEntry>,
}

#[derive(Serialize)]
pub struct ProviderHealthEntry {
    pub provider: String,
    pub health_state: String,
    pub consecutive_failures: u32,
    pub in_flight: u32,
    pub max_concurrency: Option<u32>,
    pub load_score: Option<f32>,
    pub backoff_ms: u64,
    pub available: bool,
    pub last_failure_ago_ms: Option<u64>,
    pub rate_limited: bool,
    /// Current balance for this provider, if it supports balance tracking.
    /// Serialized as `{"currency": "msats"|"sats"|"usd_micro", "amount": N}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<crate::providers::provider_trait::CurrencyAmount>,
}

#[derive(Serialize)]
pub struct HealthOverviewResponse {
    pub providers: Vec<ProviderHealthEntry>,
    pub provider_count: usize,
    pub unhealthy_count: usize,
    pub degraded_count: usize,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub providers: Vec<ProviderMetrics>,
    pub recent_events: Vec<serde_json::Value>,
    pub total_requests: u64,
    pub total_successes: u64,
    pub total_failures: u64,
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

#[derive(Serialize)]
pub struct ConfigResponse {
    pub server: ServerConfigResponse,
    pub database: DatabaseConfigResponse,
    pub auth: Option<AuthConfigResponse>,
}

#[derive(Serialize)]
pub struct ServerConfigResponse {
    pub host: String,
    pub port: u16,
}

#[derive(Serialize)]
pub struct DatabaseConfigResponse {
    pub url: String,
}

#[derive(Serialize)]
pub struct AuthConfigResponse {
    pub enabled: bool,
    pub allowed_pubkeys: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub provider_type: String,
    pub created_at: String,
    pub updated_at: String,
    /// Live health+metrics for this provider.
    /// Only present when the router has the provider loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ProviderHealthEntry>,
}

#[derive(Serialize)]
pub struct ListProvidersResponse {
    pub providers: Vec<ProviderResponse>,
}

#[derive(Serialize)]
pub struct ProviderCreateResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ProviderDeleteResponse {
    pub deleted: bool,
    pub slug: String,
}

#[derive(Serialize)]
pub struct ProviderMetricsResponse {
    pub p90_ttft_ms: Option<u32>,
    pub p90_output_tokens_per_second: Option<f32>,
    pub p90_input_tokens_per_second: Option<f32>,
    pub avg_latency_ms: Option<f32>,
    pub success_rate: Option<f32>,
}

#[derive(Serialize)]
pub struct RoutingConfigProvider {
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub list_url: String,
    pub metrics: ProviderMetricsResponse,
}

#[derive(Serialize)]
pub struct RoutingConfig {
    pub name: String,
    pub strategy: String,
    pub providers: Vec<RoutingConfigProvider>,
    pub provider_count: usize,
}

#[derive(Serialize)]
pub struct RouterConfigResponse {
    pub routing_configs: Vec<RoutingConfig>,
}

#[derive(Deserialize)]
pub struct RoutingConfigCreateRequest {
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
}

#[derive(Deserialize)]
pub struct RoutingConfigUpdateRequest {
    pub name: Option<String>,
    pub strategy: Option<String>,
    pub health_check_enabled: Option<bool>,
    pub health_check_interval_seconds: Option<i32>,
    pub health_check_timeout_seconds: Option<i32>,
}

#[derive(Deserialize)]
pub struct RoutingConfigProviderCreateRequest {
    pub routing_config_id: i64,
    pub provider_id: i64,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct RoutingConfigProviderUpdateRequest {
    pub model: Option<String>,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Serialize)]
pub struct RoutingConfigFullResponse {
    pub id: i64,
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
    pub created_at: String,
    pub updated_at: String,
    pub providers: Vec<RoutingConfigProviderFullResponse>,
}

#[derive(Serialize)]
pub struct RoutingConfigProviderFullResponse {
    pub id: i64,
    pub routing_config_id: i64,
    pub provider_id: i64,
    pub provider_name: String,
    pub provider_slug: String,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct SyncModelsResponse {
    pub provider: String,
    pub models: Vec<serde_json::Value>,
    pub total_count: usize,
}

pub async fn list_models(State(state): State<std::sync::Arc<AppState>>) -> Json<ModelsListResponse> {
    let providers = state.config.router.get_providers().await;
    let routing_configs = state.config.db.list_routing_configs().await.unwrap_or_default();
    let mut all_models = Vec::new();

    let payments_enabled = state.payments_state.is_some();

    // Add routing configs (routing engines) as models
    for rc in &routing_configs {
        all_models.push(ModelEntry {
            id: rc.name.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: rc.name.clone(),
            pricing: None,
        });
    }

    // Add actual models from providers with provider slug prefix
    for provider in &providers {
        let provider_slug = provider.slug();
        
        match provider.list_models().await {
            Ok(models) => {
                for model in models {
                    let full_id = format!("{}/{}", provider_slug, model.id);

                    // Resolve pricing for this model (even when payments disabled, show defaults)
                    let pricing = if payments_enabled {
                        if let Some(ref ps) = state.payments_state {
                            let p = ps.pricing_resolver.resolve(&model.id).await;
                            if !p.is_advertised {
                                continue; // skip unadvertised models
                            }
                            Some(ModelPricing {
                                prompt: p.price_per_1m_input_sats,
                                completion: p.price_per_1m_output_sats,
                                request: p.price_per_request_sats,
                                unit: "1M tokens".to_string(),
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    all_models.push(ModelEntry {
                        id: full_id,
                        object: model.object,
                        created: model.created as i64,
                        owned_by: model.owned_by,
                        pricing,
                    });
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Truncate error message to avoid logging huge JSON responses
                let short_error = if error_msg.len() > 200 {
                    format!("{}... (truncated)", &error_msg[..200])
                } else {
                    error_msg
                };
                tracing::warn!(
                    provider = provider.name(),
                    error = %short_error,
                    "Failed to list models from provider"
                );
            }
        }
    }

    Json(ModelsListResponse {
        object: "list".to_string(),
        data: all_models,
    })
}

/// A model entry conforming to RIP-01 / RIP-05: includes pricing in sats.
#[derive(Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
    /// Pricing structure per RIP-05. None if payments are disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

/// Pricing breakdown per RIP-05, in sats.
#[derive(Serialize)]
pub struct ModelPricing {
    /// Sats per 1M input/prompt tokens.
    pub prompt: i64,
    /// Sats per 1M output/completion tokens.
    pub completion: i64,
    /// Sats per request.
    pub request: i64,
    /// Unit for token pricing (always "1M tokens").
    pub unit: String,
}

#[derive(Serialize)]
pub struct ModelsListResponse {
    pub object: String,
    pub data: Vec<ModelEntry>,
}

pub async fn get_router_config(State(state): State<std::sync::Arc<AppState>>) -> Json<RouterConfigResponse> {
    let providers = state.config.router.get_providers().await;
    let db_providers = state.config.db.list_providers().await.unwrap_or_default();
    
    // Build provider info map for metrics lookup
    let mut provider_list = Vec::new();
    
    for provider in &providers {
        if let Some(db_provider) = db_providers.iter().find(|db| db.slug == provider.slug()) {
            let metrics = state.metrics_store.get_provider_summary(provider.name()).await;
            
            provider_list.push(RoutingConfigProvider {
                name: provider.name().to_string(),
                slug: provider.slug().to_string(),
                base_url: db_provider.base_url.clone(),
                list_url: format!("{}/v1/models", db_provider.base_url),
                metrics: ProviderMetricsResponse {
                    p90_ttft_ms: metrics.p90_ttft,
                    p90_output_tokens_per_second: metrics.p90_output_tokens_per_second,
                    p90_input_tokens_per_second: metrics.p90_input_tokens_per_second,
                    avg_latency_ms: metrics.avg_latency,
                    success_rate: metrics.success_rate,
                },
            });
        }
    }

    // For now, return all providers under a single "default" routing config
    // In the future, this could be extended to support multiple routing configs (model aliases)
    Json(RouterConfigResponse {
        routing_configs: vec![RoutingConfig {
            name: "default".to_string(),
            strategy: "round_robin".to_string(),
            providers: provider_list,
            provider_count: providers.len(),
        }],
    })
}

#[axum::debug_handler]
pub async fn list_routing_configs(State(state): State<std::sync::Arc<AppState>>) -> Json<Vec<RoutingConfigFullResponse>> {
    let configs = match state.config.db.list_routing_configs().await {
        Ok(configs) => configs,
        Err(e) => {
            tracing::error!("Failed to list routing configs: {}", e);
            return Json(vec![]);
        }
    };

    let db_providers = match state.config.db.list_providers().await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!("Failed to list providers: {}", e);
            return Json(vec![]);
        }
    };

    let mut response = Vec::new();
    for config in configs {
        let providers = match state.config.db.list_routing_config_providers_for_config(config.id).await {
            Ok(providers) => providers,
            Err(e) => {
                tracing::error!("Failed to list providers for routing config {}: {}", config.id, e);
                continue;
            }
        };

        let provider_responses: Vec<RoutingConfigProviderFullResponse> = providers
            .iter()
            .map(|rp| {
                let provider_name = db_providers
                    .iter()
                    .find(|p| p.id == rp.provider_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "Unknown".to_string());
                let provider_slug = db_providers
                    .iter()
                    .find(|p| p.id == rp.provider_id)
                    .map(|p| p.slug.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                RoutingConfigProviderFullResponse {
                    id: rp.id,
                    routing_config_id: rp.routing_config_id,
                    provider_id: rp.provider_id,
                    provider_name,
                    provider_slug,
                    model: rp.model.clone(),
                    weight: rp.weight,
                    is_active: rp.is_active,
                    created_at: rp.created_at.clone(),
                    updated_at: rp.updated_at.clone(),
                }
            })
            .collect();

        response.push(RoutingConfigFullResponse {
            id: config.id,
            name: config.name,
            strategy: config.strategy,
            health_check_enabled: config.health_check_enabled,
            health_check_interval_seconds: config.health_check_interval_seconds,
            health_check_timeout_seconds: config.health_check_timeout_seconds,
            created_at: config.created_at,
            updated_at: config.updated_at,
            providers: provider_responses,
        });
    }

    Json(response)
}

#[axum::debug_handler]
pub async fn create_routing_config(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<RoutingConfigCreateRequest>,
) -> Result<Json<RoutingConfigFullResponse>, (axum::http::StatusCode, String)> {
    let config = crate::db::NewRoutingConfig {
        name: request.name.clone(),
        strategy: request.strategy.clone(),
        health_check_enabled: request.health_check_enabled,
        health_check_interval_seconds: request.health_check_interval_seconds,
        health_check_timeout_seconds: request.health_check_timeout_seconds,
    };

    let created = match state.config.db.create_routing_config(config).await {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("Failed to create routing config: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    let full = match state.config.db.list_routing_config_providers_for_config(created.id).await {
        Ok(providers) => {
            let db_providers = match state.config.db.list_providers().await {
                Ok(providers) => providers,
                Err(e) => {
                    tracing::error!("Failed to list providers: {}", e);
                    return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
                }
            };

            let provider_responses: Vec<RoutingConfigProviderFullResponse> = providers
                .iter()
                .map(|rp| {
                    let provider_name = db_providers
                        .iter()
                        .find(|p| p.id == rp.provider_id)
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "Unknown".to_string());
                    let provider_slug = db_providers
                        .iter()
                        .find(|p| p.id == rp.provider_id)
                        .map(|p| p.slug.clone())
                        .unwrap_or_else(|| "unknown".to_string());

                    RoutingConfigProviderFullResponse {
                        id: rp.id,
                        routing_config_id: rp.routing_config_id,
                        provider_id: rp.provider_id,
                        provider_name,
                        provider_slug,
                        model: rp.model.clone(),
                        weight: rp.weight,
                        is_active: rp.is_active,
                        created_at: rp.created_at.clone(),
                        updated_at: rp.updated_at.clone(),
                    }
                })
                .collect();

            RoutingConfigFullResponse {
                id: created.id,
                name: created.name,
                strategy: created.strategy,
                health_check_enabled: created.health_check_enabled,
                health_check_interval_seconds: created.health_check_interval_seconds,
                health_check_timeout_seconds: created.health_check_timeout_seconds,
                created_at: created.created_at,
                updated_at: created.updated_at,
                providers: provider_responses,
            }
        }
        Err(e) => {
            tracing::error!("Failed to list providers for new routing config: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    Ok(Json(full))
}

#[axum::debug_handler]
pub async fn update_routing_config(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<RoutingConfigUpdateRequest>,
) -> Result<Json<RoutingConfigFullResponse>, (axum::http::StatusCode, String)> {
    let updates = crate::db::UpdateRoutingConfig {
        name: request.name.clone(),
        strategy: request.strategy.clone(),
        health_check_enabled: request.health_check_enabled,
        health_check_interval_seconds: request.health_check_interval_seconds,
        health_check_timeout_seconds: request.health_check_timeout_seconds,
    };

    let updated = match state.config.db.update_routing_config(id, updates).await {
        Ok(config) => config,
        Err(e) => {
            tracing::error!("Failed to update routing config: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let providers = match state.config.db.list_routing_config_providers_for_config(updated.id).await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!("Failed to list providers for routing config {}: {}", updated.id, e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    let db_providers = match state.config.db.list_providers().await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!("Failed to list providers: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    let provider_responses: Vec<RoutingConfigProviderFullResponse> = providers
        .iter()
        .map(|rp| {
            let provider_name = db_providers
                .iter()
                .find(|p| p.id == rp.provider_id)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let provider_slug = db_providers
                .iter()
                .find(|p| p.id == rp.provider_id)
                .map(|p| p.slug.clone())
                .unwrap_or_else(|| "unknown".to_string());

            RoutingConfigProviderFullResponse {
                id: rp.id,
                routing_config_id: rp.routing_config_id,
                provider_id: rp.provider_id,
                provider_name,
                provider_slug,
                model: rp.model.clone(),
                weight: rp.weight,
                is_active: rp.is_active,
                created_at: rp.created_at.clone(),
                updated_at: rp.updated_at.clone(),
            }
        })
        .collect();

    Ok(Json(RoutingConfigFullResponse {
        id: updated.id,
        name: updated.name,
        strategy: updated.strategy,
        health_check_enabled: updated.health_check_enabled,
        health_check_interval_seconds: updated.health_check_interval_seconds,
        health_check_timeout_seconds: updated.health_check_timeout_seconds,
        created_at: updated.created_at,
        updated_at: updated.updated_at,
        providers: provider_responses,
    }))
}

#[axum::debug_handler]
pub async fn delete_routing_config(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let deleted = match state.config.db.delete_routing_config(id).await {
        Ok(deleted) => deleted,
        Err(e) => {
            tracing::error!("Failed to delete routing config: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    if !deleted {
        return Err((axum::http::StatusCode::NOT_FOUND, "Routing config not found".to_string()));
    }

    Ok(Json(serde_json::json!({
        "message": "Routing config deleted successfully",
        "id": id
    })))
}

#[axum::debug_handler]
pub async fn create_routing_config_provider(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<RoutingConfigProviderCreateRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let rcp = crate::db::NewRoutingConfigProvider {
        routing_config_id: request.routing_config_id,
        provider_id: request.provider_id,
        model: request.model.clone(),
        weight: request.weight,
        is_active: request.is_active,
    };

    match state.config.db.create_routing_config_provider(rcp).await {
        Ok(_) => {
            state.config.router.reload_config().await
                .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(Json(serde_json::json!({
                "message": "Routing config provider created successfully"
            })))
        },
        Err(e) => {
            tracing::error!("Failed to create routing config provider: {}", e);
            Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

#[axum::debug_handler]
pub async fn update_routing_config_provider(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<RoutingConfigProviderUpdateRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let updates = crate::db::UpdateRoutingConfigProvider {
        model: request.model.clone(),
        weight: request.weight,
        is_active: request.is_active,
    };

    match state.config.db.update_routing_config_provider(id, updates).await {
        Ok(_) => {
            state.config.router.reload_config().await
                .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(Json(serde_json::json!({
                "message": "Routing config provider updated successfully"
            })))
        },
        Err(e) => {
            tracing::error!("Failed to update routing config provider: {}", e);
            Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

#[axum::debug_handler]
pub async fn delete_routing_config_provider(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let deleted = match state.config.db.delete_routing_config_provider(id).await {
        Ok(deleted) => deleted,
        Err(e) => {
            tracing::error!("Failed to delete routing config provider: {}", e);
            return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    if !deleted {
        return Err((axum::http::StatusCode::NOT_FOUND, "Routing config provider not found".to_string()));
    }

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "message": "Routing config provider deleted successfully",
        "id": id
    })))
}

/// Build a ProviderHealthEntry for a live provider by looking up metrics,
/// health state, and balance from the AppState.
pub async fn build_provider_health_entry(
    state: &std::sync::Arc<AppState>,
    provider: &dyn crate::providers::Provider,
) -> ProviderHealthEntry {
    use std::time::Instant;
    let name = provider.name();
    let health = state.metrics_store.get_provider_health(name).await;
    let failures = state.metrics_store.get_recent_failures(name).await;
    let backoff = state.metrics_store.get_provider_backoff(name).await;
    let load_score = state.metrics_store.get_provider_load_score(name).await;
    let available = state.metrics_store.is_provider_available(name).await;
    let (in_flight, max_concurrency): (u32, Option<u32>) =
        state.metrics_store.get_provider_load(name).await.unwrap_or((0, None));

    let now = Instant::now();
    let last_failure_ago_ms = {
        state.metrics_store.events.lock().ok().and_then(|ev| {
            ev.iter().rev()
                .find(|e| e.provider == name && matches!(e.event, crate::metrics::MetricsEvent::Failure(_)))
                .map(|_| now.elapsed().as_millis() as u64)
        })
    };
    let rate_limited = {
        state.metrics_store.events.lock().ok().is_some_and(|ev| {
            ev.iter().rev().any(|e| {
                e.provider == name && matches!(&e.event, crate::metrics::MetricsEvent::Failure(d) if d.error_type == crate::metrics::ErrorType::RateLimit)
            })
        })
    };

    ProviderHealthEntry {
        provider: name.to_string(),
        health_state: format!("{:?}", health).to_lowercase(),
        consecutive_failures: failures,
        in_flight,
        max_concurrency,
        load_score,
        backoff_ms: backoff.as_millis() as u64,
        available,
        last_failure_ago_ms,
        rate_limited,
        balance: state.metrics_store.get_balance(name).await,
    }
}

pub async fn get_metrics(State(state): State<std::sync::Arc<AppState>>) -> Json<MetricsResponse> {
    let providers = state.config.router.get_providers().await;
    let mut provider_metrics = Vec::new();

    for provider in &providers {
        let provider_name = provider.name();
        let summary = state.metrics_store.get_provider_summary(provider_name).await;
        let health = state.metrics_store.get_provider_health(provider_name).await;
        let failures = state.metrics_store.get_recent_failures(provider_name).await;
        let backoff = state.metrics_store.get_provider_backoff(provider_name).await;
        let load_score = state.metrics_store.get_provider_load_score(provider_name).await;
        let available = state.metrics_store.is_provider_available(provider_name).await;
        let (in_flight, max_concurrency): (u32, Option<u32>) = state.metrics_store.get_provider_load(provider_name).await.unwrap_or((0, None));
        
        provider_metrics.push(ProviderMetrics {
            provider: summary.provider,
            p90_tokens_per_second: summary.p90_output_tokens_per_second,
            p90_input_tokens_per_second: summary.p90_input_tokens_per_second,
            p90_ttft_ms: summary.p90_ttft,
            avg_latency_ms: summary.avg_latency,
            success_rate: summary.success_rate,
            health_state: Some(format!("{:?}", health).to_lowercase()),
            consecutive_failures: Some(failures),
            in_flight: Some(in_flight),
            max_concurrency,
            backoff_ms: Some(backoff.as_millis() as u64),
            load_score,
            available: Some(available),
            health: Some(build_provider_health_entry(&state, provider.as_ref()).await),
        });
    }

    let recent_events: Vec<serde_json::Value> = state
        .metrics_store
        .recent_events(50)
        .await
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();

    let (total_requests, total_successes, total_failures) = state.metrics_store.get_total_requests();

    Json(MetricsResponse {
        providers: provider_metrics,
        recent_events,
        total_requests,
        total_successes,
        total_failures,
    })
}

pub async fn get_health_overview(State(state): State<std::sync::Arc<AppState>>) -> Json<HealthOverviewResponse> {
    let providers = state.config.router.get_providers().await;
    let mut health_entries = Vec::new();
    let mut unhealthy = 0;
    let mut degraded = 0;

    for provider in &providers {
        let entry = build_provider_health_entry(&state, provider.as_ref()).await;
        if entry.health_state == "unhealthy" { unhealthy += 1; }
        else if entry.health_state == "degraded" { degraded += 1; }
        health_entries.push(entry);
    }

    Json(HealthOverviewResponse {
        providers: health_entries,
        provider_count: providers.len(),
        unhealthy_count: unhealthy,
        degraded_count: degraded,
    })
}

pub async fn get_metrics_history(State(state): State<std::sync::Arc<AppState>>) -> Json<serde_json::Value> {
    let history = state.metrics_store.get_history().await;
    Json(serde_json::to_value(history).unwrap_or_default())
}

#[axum::debug_handler]
pub async fn chat_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(user): Extension<crate::db::User>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<axum::response::Response, (axum::http::StatusCode, String)> {
    if request.stream.unwrap_or(false) {
        let stream_response = chat_completions_stream(State(state), Extension(user), Json(request)).await;
        Ok(stream_response.into_response())
    } else {
        let response = chat_completions_handler(State(state), Extension(user), Json(request)).await?;
        Ok(response.into_response())
    }
}

#[axum::debug_handler]
pub async fn chat_completions_handler(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(user): Extension<crate::db::User>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Json<ChatCompletionResponse>, (axum::http::StatusCode, String)> {
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

    match state.config.router.chat_completions(&request).await {
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
    Extension(user): Extension<crate::db::User>,
    Json(request): Json<ChatCompletionRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static>, (axum::http::StatusCode, String)> {
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

    let model = request.model.clone();
    let stream: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send + 'static>> = 
        match state.config.router.chat_completions_stream(&request).await {
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

#[derive(Deserialize)]
pub struct ProviderCreateRequest {
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: String,
    pub provider_type: String,
}

#[axum::debug_handler]
pub async fn list_providers(State(state): State<std::sync::Arc<AppState>>) -> Json<ListProvidersResponse> {
    let providers = match state.config.db.list_providers().await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!("Failed to list providers from DB: {}", e);
            return Json(ListProvidersResponse { providers: vec![] });
        }
    };

    // Build slug→health-entry map from live router providers
    let live_providers = state.config.router.get_providers().await;
    let mut health_by_slug: std::collections::HashMap<String, ProviderHealthEntry> =
        std::collections::HashMap::new();
    for provider in &live_providers {
        let entry = build_provider_health_entry(&state, provider.as_ref()).await;
        health_by_slug.insert(provider.slug().to_string(), entry);
    }

    let providers_list: Vec<ProviderResponse> = providers
        .into_iter()
        .map(|p| ProviderResponse {
            id: p.id,
            name: p.name.clone(),
            slug: p.slug.clone(),
            base_url: p.base_url,
            provider_type: p.provider_type.as_str().to_string(),
            created_at: p.created_at,
            updated_at: p.updated_at,
            health: health_by_slug.remove(&p.slug),
        })
        .collect();

    Json(ListProvidersResponse {
        providers: providers_list,
    })
}

#[axum::debug_handler]
pub async fn create_provider(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<ProviderCreateRequest>,
) -> Result<Json<ProviderCreateResponse>, (axum::http::StatusCode, String)> {
    use crate::db::ProviderType;

    let provider_type = ProviderType::from_str(&request.provider_type)
        .unwrap_or(ProviderType::OpenAi);

    let new_provider = crate::db::NewProvider {
        name: &request.name,
        slug: &request.slug,
        base_url: &request.base_url,
        api_key: Some(&request.api_key),
        provider_type: Some(provider_type),
    };

    let created = state.config.db.create_provider(new_provider).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    
    tracing::info!(
        provider_name = request.name,
        "Provider added and config reloaded successfully"
    );

    Ok(Json(ProviderCreateResponse {
        id: created.id,
        name: created.name,
        slug: created.slug,
        base_url: created.base_url,
        created_at: created.created_at,
    }))
}

#[axum::debug_handler]
pub async fn delete_provider(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<ProviderDeleteResponse>, (axum::http::StatusCode, String)> {
    tracing::info!(provider_slug = slug, "Deleting provider");
    
    // Delete from database
    sqlx::query("DELETE FROM providers WHERE slug = ?")
        .bind(&slug)
        .execute(&state.config.db.pool)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Provider deleted and config reloaded");

    Ok(Json(ProviderDeleteResponse {
        deleted: true,
        slug,
    }))
}

#[derive(Deserialize)]
pub struct ProviderUpdateRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub provider_type: Option<String>,
}

#[axum::debug_handler]
pub async fn update_provider(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<ProviderUpdateRequest>,
) -> Result<Json<ProviderResponse>, (axum::http::StatusCode, String)> {
    use crate::db::ProviderType;

    let provider = state.config.db.get_provider_by_slug(&slug).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", slug)))?;

    let provider_type = request.provider_type.as_deref()
        .and_then(ProviderType::from_str);

    let api_key = request.api_key.as_deref().map(Some);

    let updated = state.config.db.update_provider(provider.id, crate::db::UpdateProvider {
        name: request.name.as_deref(),
        slug: request.slug.as_deref(),
        base_url: request.base_url.as_deref(),
        api_key,
        provider_type,
    }).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Provider updated and config reloaded");

    Ok(Json(ProviderResponse {
        id: updated.id,
        name: updated.name,
        slug: updated.slug,
        base_url: updated.base_url,
        provider_type: updated.provider_type.as_str().to_string(),
        created_at: updated.created_at,
        updated_at: updated.updated_at,
        health: None,
    }))
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
    State(state): State<std::sync::Arc<AppState>>,
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
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<SyncModelsResponse>, (axum::http::StatusCode, String)> {
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

            Ok(Json(SyncModelsResponse {
                provider: provider_slug,
                models: model_details,
                total_count: models.len(),
            }))
        }
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ============================================================================
// Provider Models Listing
// ============================================================================

#[derive(Serialize)]
pub struct ProviderModelItem {
    pub id: String,
    pub created: u32,
    pub owned_by: String,
}

#[derive(Serialize)]
pub struct ProviderModelsResponse {
    pub provider: String,
    pub models: Vec<ProviderModelItem>,
    pub total_count: usize,
}

#[axum::debug_handler]
pub async fn list_provider_models(
    Path(provider_slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<ProviderModelsResponse>, (axum::http::StatusCode, String)> {
    let providers = state.config.router.get_providers().await;
    let provider = providers
        .iter()
        .find(|p| p.slug() == provider_slug)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", provider_slug)))?;

    match provider.list_models().await {
        Ok(models) => {
            let model_items: Vec<ProviderModelItem> = models
                .into_iter()
                .map(|m| ProviderModelItem {
                    id: m.id,
                    created: m.created,
                    owned_by: m.owned_by,
                })
                .collect();

            let total = model_items.len();
            Ok(Json(ProviderModelsResponse {
                provider: provider_slug,
                models: model_items,
                total_count: total,
            }))
        }
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

// ============================================================================
// Responses API Handlers
// ============================================================================

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

    match state.config.router.responses(&request).await {
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

// ============================================================================
// Model Pricing Admin API
// ============================================================================

#[derive(Serialize)]
pub struct ModelPricingResponse {
    pub id: i64,
    pub model_name: String,
    pub is_advertised: bool,
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct ModelPricingCreateRequest {
    pub model_name: String,
    #[serde(default = "default_true")]
    pub is_advertised: bool,
    #[serde(default)]
    pub is_free: bool,
    pub price_per_1m_input_sats: Option<i64>,
    pub price_per_1m_output_sats: Option<i64>,
    pub price_per_request_sats: Option<i64>,
    pub context_window: Option<i32>,
    pub max_output_tokens: Option<i32>,
}

#[derive(Deserialize)]
pub struct ModelPricingUpdateRequest {
    pub is_advertised: Option<bool>,
    pub is_free: Option<bool>,
    pub price_per_1m_input_sats: Option<Option<i64>>,
    pub price_per_1m_output_sats: Option<Option<i64>>,
    pub price_per_request_sats: Option<Option<i64>>,
    pub context_window: Option<Option<i32>>,
    pub max_output_tokens: Option<Option<i32>>,
}

fn default_true() -> bool { true }

#[axum::debug_handler]
pub async fn list_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<ModelPricingResponse>> {
    let rows = state.config.db.list_model_pricings().await.unwrap_or_default();
    Json(
        rows.into_iter()
            .map(|r| ModelPricingResponse {
                id: r.id,
                model_name: r.model_name,
                is_advertised: r.is_advertised,
                is_free: r.is_free,
                price_per_1m_input_sats: r.price_per_1m_input_sats,
                price_per_1m_output_sats: r.price_per_1m_output_sats,
                price_per_request_sats: r.price_per_request_sats,
                context_window: r.context_window,
                max_output_tokens: r.max_output_tokens,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
    )
}

#[axum::debug_handler]
pub async fn create_model_pricing(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<ModelPricingCreateRequest>,
) -> Result<Json<ModelPricingResponse>, (StatusCode, String)> {
    let mp = crate::db::NewModelPricing {
        model_name: &req.model_name,
        is_advertised: req.is_advertised,
        is_free: req.is_free,
        price_per_1m_input_sats: req.price_per_1m_input_sats,
        price_per_1m_output_sats: req.price_per_1m_output_sats,
        price_per_request_sats: req.price_per_request_sats,
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
    };

    match state.config.db.create_model_pricing(mp).await {
        Ok(r) => Ok(Json(ModelPricingResponse {
            id: r.id,
            model_name: r.model_name,
            is_advertised: r.is_advertised,
            is_free: r.is_free,
            price_per_1m_input_sats: r.price_per_1m_input_sats,
            price_per_1m_output_sats: r.price_per_1m_output_sats,
            price_per_request_sats: r.price_per_request_sats,
            context_window: r.context_window,
            max_output_tokens: r.max_output_tokens,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[axum::debug_handler]
pub async fn update_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<ModelPricingUpdateRequest>,
) -> Result<Json<ModelPricingResponse>, (StatusCode, String)> {
    let updates = crate::db::UpdateModelPricing {
        is_advertised: req.is_advertised,
        is_free: req.is_free,
        price_per_1m_input_sats: req.price_per_1m_input_sats,
        price_per_1m_output_sats: req.price_per_1m_output_sats,
        price_per_request_sats: req.price_per_request_sats,
        context_window: req.context_window,
        max_output_tokens: req.max_output_tokens,
    };

    match state.config.db.update_model_pricing(&model_name, updates).await {
        Ok(r) => Ok(Json(ModelPricingResponse {
            id: r.id,
            model_name: r.model_name,
            is_advertised: r.is_advertised,
            is_free: r.is_free,
            price_per_1m_input_sats: r.price_per_1m_input_sats,
            price_per_1m_output_sats: r.price_per_1m_output_sats,
            price_per_request_sats: r.price_per_request_sats,
            context_window: r.context_window,
            max_output_tokens: r.max_output_tokens,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[axum::debug_handler]
pub async fn delete_model_pricing(
    Path(model_name): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.config.db.delete_model_pricing(&model_name).await {
        Ok(true) => Ok(Json(serde_json::json!({"deleted": true, "model_name": model_name}))),
        Ok(false) => Err((StatusCode::NOT_FOUND, "Model pricing not found".to_string())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::server::create_test_app;
    use crate::auth::admin::{SessionStore, setup_first_user};
    use crate::config::{Config, ServerConfig, DatabaseConfig};
    use crate::db::{Database, NewUser, UserType};
    use crate::metrics::{MetricsEmitter, MetricsStore};
    use crate::state::AppState;
    use axum::{body::Body, http::{Request, header}, Router};
    use serde_json::json;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    async fn setup_test_state() -> (Arc<AppState>, MetricsStore) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        
        let metrics_store = MetricsStore::new(1000);
        
        let config = Config {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
                admin_ui_path: "/app/admin/dist".to_string(),
            },
            database: DatabaseConfig {
                url: "sqlite::memory:".to_string(),
            },
            auth: None,
            payments: None,
        };

        let app_config = crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router: Arc::new(crate::router::engine::Router::new(
                metrics_store.clone(),
                Arc::new(db.clone()),
            )),
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
            admin_ui_path: "/app/admin/dist".to_string(),
        };

        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: app_config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.clone().into(),
            session_store,
            db: Arc::new(db),
            payments_state: None,
        });

        (state, metrics_store)
    }

    async fn setup_admin_user(state: &Arc<AppState>) -> String {
        use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
        use rand::rngs::OsRng;
        
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(b"password123", &salt)
            .unwrap()
            .to_string();

        state.db.create_user(NewUser {
            username: Some("admin"),
            password_hash: Some(&password_hash),
            external_id: None,
            user_type: UserType::Internal,
            is_admin: true,
        }).await.unwrap();

        state.session_store.create("admin", true, 86400).await
    }

    #[tokio::test]
    async fn test_health_check() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_v1_models() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .oneshot(Request::builder().uri("/v1/models").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_setup_status() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/api/setup/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_setup() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/setup")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "username": "admin",
                        "password": "password123"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_login() {
        let (state, _) = setup_test_state().await;
        setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/login")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "username": "admin",
                        "password": "password123"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_protected_routes_require_auth() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/api/providers").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn test_protected_routes_with_auth() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_keys_crud() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        // Create API key
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/api-keys")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "name": "test-key"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(create_response.status(), 200);

        // List API keys
        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/api-keys")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(list_response.status(), 200);
    }

    #[tokio::test]
    async fn test_chat_completion_requires_auth() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/chat/completions")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "model": "test",
                        "messages": [{"role": "user", "content": "hello"}]
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 401); // Auth middleware returns 401 when auth is missing
    }

    #[tokio::test]
    async fn test_api_auth_status() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_logout() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/logout")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_providers_crud() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        // Create provider
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "name": "test-provider",
                        "slug": "test",
                        "base_url": "http://localhost:8080",
                        "api_key": "test-key",
                        "provider_type": "openai"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(create_response.status(), 200);

        // List providers
        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(list_response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_config() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_metrics() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}

// User Management Types and Handlers

#[derive(Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: Option<String>,
    pub external_id: Option<String>,
    pub user_type: String,
    pub is_admin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct UserDetailResponse {
    pub user: UserResponse,
    pub api_keys: Vec<UserApiKeyResponse>,
}

#[derive(Serialize)]
pub struct UserApiKeyResponse {
    pub id: i64,
    pub name: String,
    pub last_four: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub external_id: Option<String>,
    pub user_type: String,
    pub is_admin: bool,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub is_admin: Option<bool>,
}

#[derive(Serialize)]
pub struct UserCreateResponse {
    pub message: String,
    pub user: UserResponse,
}

#[derive(Serialize)]
pub struct UserDeleteResponse {
    pub message: String,
}

#[axum::debug_handler]
pub async fn list_users(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<UserResponse>> {
    let users = match state.db.list_users().await {
        Ok(users) => users,
        Err(e) => {
            tracing::error!("Failed to list users from DB: {}", e);
            return Json(vec![]);
        }
    };

    let users_list: Vec<UserResponse> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id,
            username: u.username,
            external_id: u.external_id,
            user_type: match u.user_type {
                UserType::Internal => "internal".to_string(),
                UserType::Nostr => "nostr".to_string(),
                UserType::OAuth => "oauth".to_string(),
            },
            is_admin: u.is_admin,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .collect();

    Json(users_list)
}

#[axum::debug_handler]
pub async fn create_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<UserCreateResponse>, (axum::http::StatusCode, String)> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use rand::rngs::OsRng;

    if request.username.is_none() && request.external_id.is_none() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "Either username or external_id is required".to_string()));
    }

    if let Some(username) = &request.username {
        if state.db.get_user_by_username(username).await.unwrap_or(None).is_some() {
            return Err((axum::http::StatusCode::BAD_REQUEST, format!("User '{}' already exists", username)));
        }
    }

    let user_type = match request.user_type.as_str() {
        "internal" => UserType::Internal,
        "nostr" => UserType::Nostr,
        "oauth" => UserType::OAuth,
        _ => UserType::Internal,
    };

    let password_hash = if let Some(password) = &request.password {
        if user_type == UserType::Internal {
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            Some(
                argon2
                    .hash_password(password.as_bytes(), &salt)
                    .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                    .to_string()
            )
        } else {
            None
        }
    } else {
        None
    };

    let new_user = crate::db::NewUser {
        username: request.username.as_deref(),
        password_hash: password_hash.as_deref(),
        external_id: request.external_id.as_deref(),
        user_type,
        is_admin: request.is_admin,
    };

    let user = state.db.create_user(new_user)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: user.id,
        username: user.username,
        external_id: user.external_id,
        user_type: match user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: user.is_admin,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    Ok(Json(UserCreateResponse {
        message: "User created successfully".to_string(),
        user: user_response,
    }))
}

#[axum::debug_handler]
pub async fn update_user(
    Path(id): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, (axum::http::StatusCode, String)> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use rand::rngs::OsRng;

    let user_id: i64 = id.parse()
        .map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "Invalid user ID".to_string()))?;

    let existing_user = state.db.get_user_by_id(user_id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "User not found".to_string()))?;

    if let Some(new_username) = &request.username {
        if Some(new_username.as_str()) != existing_user.username.as_deref()
            && state.db.get_user_by_username(new_username).await.unwrap_or(None).is_some() {
                return Err((axum::http::StatusCode::BAD_REQUEST, format!("User '{}' already exists", new_username)));
            }
    }

    let mut updates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(new_username) = &request.username {
        updates.push("username = ?".to_string());
        bindings.push(new_username.clone());
    }

    if let Some(new_password) = &request.password {
        if existing_user.user_type == UserType::Internal {
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let hash = argon2
                .hash_password(new_password.as_bytes(), &salt)
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .to_string();
            updates.push("password_hash = ?".to_string());
            bindings.push(hash);
        }
    }

    if request.is_admin.is_some() {
        updates.push("is_admin = ?".to_string());
    }

    if updates.is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "No updates provided".to_string()));
    }

    let mut query = format!("UPDATE users SET updated_at = CURRENT_TIMESTAMP, {}", updates.join(", "));
    query.push_str(" WHERE id = ? RETURNING *");

    let mut query_builder = sqlx::query_as::<_, crate::db::User>(&query);
    
    for binding in &bindings {
        query_builder = query_builder.bind(binding);
    }
    
    if let Some(is_admin) = request.is_admin {
        query_builder = query_builder.bind(is_admin);
    }
    
    query_builder = query_builder.bind(user_id);

    let updated_user = query_builder
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: updated_user.id,
        username: updated_user.username,
        external_id: updated_user.external_id,
        user_type: match updated_user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: updated_user.is_admin,
        created_at: updated_user.created_at,
        updated_at: updated_user.updated_at,
    };

    Ok(Json(user_response))
}

#[axum::debug_handler]
pub async fn delete_user(
    Path(id): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<UserDeleteResponse>, (axum::http::StatusCode, String)> {
    let user_id: i64 = id.parse()
        .map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "Invalid user ID".to_string()))?;

    state.db.delete_user(user_id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(UserDeleteResponse {
        message: "User deleted successfully".to_string(),
    }))
}

#[axum::debug_handler]
pub async fn get_user(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<UserDetailResponse>, (axum::http::StatusCode, String)> {
    let user = state.db.get_user_by_id(id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let api_keys = state.db.list_api_keys_for_user(id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: user.id,
        username: user.username,
        external_id: user.external_id,
        user_type: match user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: user.is_admin,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    let api_keys_response: Vec<UserApiKeyResponse> = api_keys.into_iter().map(|k| {
        UserApiKeyResponse {
            id: k.id,
            name: k.name,
            last_four: k.last_four,
            created_at: k.created_at,
            expires_at: k.expires_at.map(|e| e.to_string()),
            is_active: k.is_active,
        }
    }).collect();

    Ok(Json(UserDetailResponse {
        user: user_response,
        api_keys: api_keys_response,
    }))
}

#[derive(Serialize)]
pub struct ConfigReloadResponse {
    pub success: bool,
    pub message: String,
    pub providers_loaded: usize,
}

#[axum::debug_handler]
pub async fn reload_config(
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<ConfigReloadResponse>, (axum::http::StatusCode, String)> {
    tracing::info!("Reloading router configuration");
    
    match state.config.router.reload_config().await {
        Ok(_) => {
            let providers = state.config.router.get_providers().await;
            tracing::info!(
                providers_count = providers.len(),
                "Configuration reloaded successfully"
            );
            Ok(Json(ConfigReloadResponse {
                success: true,
                message: "Configuration reloaded successfully".to_string(),
                providers_loaded: providers.len(),
            }))
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to reload configuration");
            Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}

// ============================================================================
// Admin Payment Management Handlers
// ============================================================================

/// GET /api/payments/balances — List all user balances (admin)
#[axum::debug_handler]
pub async fn list_all_balances(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<crate::db::UserBalanceWithUsername>> {
    let balances = state.db.list_all_user_balances().await.unwrap_or_default();
    Json(balances)
}

/// GET /api/payments/balances/:user_id — Get a single user's balance + recent transactions
#[axum::debug_handler]
pub async fn get_user_balance_details(
    Path(user_id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let balance = state.db.get_user_balance(user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let transactions = state.db.get_user_transactions(user_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let balance_msat = balance.as_ref().map(|b| b.balance_msat).unwrap_or(0);
    let lifetime_deposited = balance.as_ref().map(|b| b.lifetime_deposited_msat).unwrap_or(0);

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "balance_msat": balance_msat,
        "lifetime_deposited_msat": lifetime_deposited,
        "transactions": transactions,
    })))
}

/// POST /api/payments/credit — Admin manually credits a user's balance
#[derive(Deserialize)]
pub struct AdminCreditRequest {
    pub user_id: i64,
    pub amount_sats: u64,
    pub reason: Option<String>,
}

#[axum::debug_handler]
pub async fn admin_credit_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<AdminCreditRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount_sats == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }

    let payments = state.payments_state.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))?;

    let amount_msat = (req.amount_sats as i64) * 1000;
    let ref_id = format!("admin-credit-{}", chrono::Utc::now().timestamp_millis());
    let reason = req.reason.as_deref().unwrap_or("admin_credit");

    let new_balance = payments.balance_service
        .credit(req.user_id, amount_msat, reason, &ref_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        admin_action = "credit",
        user_id = req.user_id,
        amount_msat = amount_msat,
        new_balance_msat = new_balance,
        reason = reason,
        "Admin manual credit"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "user_id": req.user_id,
        "credited_sats": req.amount_sats,
        "new_balance_msat": new_balance,
        "new_balance_sats": new_balance / 1000,
        "reason": reason,
    })))
}

/// POST /api/payments/debit — Admin manually debits a user's balance
#[derive(Deserialize)]
pub struct AdminDebitRequest {
    pub user_id: i64,
    pub amount_sats: u64,
    pub reason: Option<String>,
}

#[axum::debug_handler]
pub async fn admin_debit_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<AdminDebitRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount_sats == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }

    let payments = state.payments_state.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))?;

    let amount_msat = (req.amount_sats as i64) * 1000;
    let ref_id = format!("admin-debit-{}", chrono::Utc::now().timestamp_millis());
    let reason = req.reason.as_deref().unwrap_or("admin_debit");

    let new_balance = payments.balance_service
        .debit(req.user_id, amount_msat, reason, &ref_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        admin_action = "debit",
        user_id = req.user_id,
        amount_msat = amount_msat,
        new_balance_msat = new_balance,
        reason = reason,
        "Admin manual debit"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "user_id": req.user_id,
        "debited_sats": req.amount_sats,
        "new_balance_msat": new_balance,
        "new_balance_sats": new_balance / 1000,
        "reason": reason,
    })))
}

/// GET /api/payments/transactions — List recent transactions (admin audit)
#[axum::debug_handler]
pub async fn list_admin_transactions(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<crate::db::BalanceTransactionRow>> {
    let txs = state.db.list_all_transactions(200).await.unwrap_or_default();
    Json(txs)
}

/// GET /api/payments/invoices — List all lightning invoices (admin)
#[axum::debug_handler]
pub async fn list_admin_invoices(
    State(state): State<std::sync::Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<Vec<crate::db::LightningInvoiceRow>> {
    let user_id = params.get("user_id").and_then(|v| v.parse::<i64>().ok());
    let invoices = state.db.list_all_lightning_invoices(user_id, 200).await.unwrap_or_default();
    Json(invoices)
}
