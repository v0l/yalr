use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

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

pub async fn get_router_config(State(state): State<std::sync::Arc<AppState>>) -> Json<RouterConfigResponse> {
    let providers = state.config.router.get_providers().await;
    let db_providers = state.config.db.list_providers().await.unwrap_or_default();

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

    Json(RouterConfigResponse {
        routing_configs: vec![RoutingConfig {
            name: "default".to_string(),
            strategy: "round_robin".to_string(),
            providers: provider_list,
            provider_count: providers.len(),
        }],
    })
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
