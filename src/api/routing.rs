use crate::state::AppState;
use axum::{extract::{Path, State}, Json};
use serde::{Deserialize, Serialize};

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

/// Reject health-check seconds that would break the health loop.
/// A 0 (or negative) timeout makes `tokio::time::timeout` elapse immediately and
/// a 0 interval panics `tokio::time::interval`, so they must be >= 1.
fn validate_health_seconds(
    interval: Option<i32>,
    timeout: Option<i32>,
) -> Result<(), (axum::http::StatusCode, String)> {
    for (field, value) in [
        ("health_check_interval_seconds", interval),
        ("health_check_timeout_seconds", timeout),
    ] {
        if let Some(v) = value {
            if v < 1 {
                return Err((
                    axum::http::StatusCode::BAD_REQUEST,
                    format!("{field} must be >= 1 (got {v})"),
                ));
            }
        }
    }
    Ok(())
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
    validate_health_seconds(
        Some(request.health_check_interval_seconds),
        Some(request.health_check_timeout_seconds),
    )?;
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
    validate_health_seconds(
        request.health_check_interval_seconds,
        request.health_check_timeout_seconds,
    )?;
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
