use crate::api::tenant::{bad_request, forbidden, internal, not_found, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{RoutingConfig, RoutingConfigProvider};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::routing_modalities::{store_modalities, RequestedModalities};

type ApiResult<T> = Result<Json<T>, TenantError>;

#[derive(Deserialize)]
pub struct RoutingConfigCreateRequest {
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
    #[serde(default)]
    pub input_modalities: Option<Vec<String>>,
    #[serde(default)]
    pub output_modalities: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct RoutingConfigUpdateRequest {
    pub name: Option<String>,
    pub strategy: Option<String>,
    pub health_check_enabled: Option<bool>,
    pub health_check_interval_seconds: Option<i32>,
    pub health_check_timeout_seconds: Option<i32>,
    #[serde(default)]
    pub input_modalities: Option<Vec<String>>,
    #[serde(default)]
    pub output_modalities: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct RoutingConfigProviderCreateRequest {
    pub routing_config_id: i64,
    #[serde(default)]
    pub provider_id: Option<i64>,
    #[serde(default, alias = "child_config_id")]
    pub config_id: Option<i64>,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct RoutingConfigProviderUpdateRequest {
    pub model: Option<String>,
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

#[derive(Serialize)]
pub struct RoutingConfigFullResponse {
    pub id: i64,
    pub name: String,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
    pub owner_user_id: Option<i64>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_modalities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_modalities: Option<Vec<String>>,
    pub providers: Vec<RoutingConfigProviderFullResponse>,
}

#[derive(Serialize)]
pub struct RoutingConfigProviderFullResponse {
    pub id: i64,
    pub routing_config_id: i64,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_name: Option<String>,
    pub model: Option<String>,
    pub weight: i32,
    pub is_active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<crate::db::ExtraBody>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
}

#[derive(Serialize)]
pub struct Message {
    pub message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
}

fn validate_health_seconds(interval: Option<i32>, timeout: Option<i32>) -> Result<(), TenantError> {
    for (field, value) in [("health_check_interval_seconds", interval), ("health_check_timeout_seconds", timeout)] {
        if let Some(v) = value.filter(|v| *v < 1) {
            return Err(bad_request(&format!("{field} must be >= 1 (got {v})")));
        }
    }
    Ok(())
}

fn member_response(
    rp: &RoutingConfigProvider,
    providers: &[crate::db::Provider],
    configs: &[RoutingConfig],
) -> RoutingConfigProviderFullResponse {
    let provider = rp.provider_id.and_then(|id| providers.iter().find(|p| p.id == id));
    let child = rp.child_config_id.and_then(|id| configs.iter().find(|c| c.id == id));
    RoutingConfigProviderFullResponse {
        id: rp.id,
        routing_config_id: rp.routing_config_id,
        kind: if rp.child_config_id.is_some() { "config" } else { "provider" },
        provider_id: rp.provider_id,
        provider_name: rp.provider_id.map(|_| provider.map_or_else(|| "Unknown".to_string(), |p| p.name.clone())),
        provider_slug: rp.provider_id.map(|_| provider.map_or_else(|| "unknown".to_string(), |p| p.slug.clone())),
        config_id: rp.child_config_id,
        config_name: rp.child_config_id.map(|_| child.map_or_else(|| "Unknown".to_string(), |c| c.name.clone())),
        model: rp.model.clone(),
        weight: rp.weight,
        is_active: rp.is_active,
        extra_body: rp.extra_body_map(),
        created_at: rp.created_at,
        updated_at: rp.updated_at,
    }
}

async fn full_response(state: &AppState, config: RoutingConfig) -> Result<RoutingConfigFullResponse, TenantError> {
    let members = state.db.list_routing_config_providers_for_config(config.id).await.map_err(internal)?;
    let providers = state.db.list_providers().await.map_err(internal)?;
    let configs = state.db.list_routing_configs().await.map_err(internal)?;
    let declared = config.declared_modalities();
    Ok(RoutingConfigFullResponse {
        id: config.id,
        input_modalities: crate::db::DeclaredModalities::names(&declared.input),
        output_modalities: crate::db::DeclaredModalities::names(&declared.output),
        name: config.name,
        strategy: config.strategy,
        health_check_enabled: config.health_check_enabled,
        health_check_interval_seconds: config.health_check_interval_seconds,
        health_check_timeout_seconds: config.health_check_timeout_seconds,
        owner_user_id: config.owner_user_id,
        created_at: config.created_at,
        updated_at: config.updated_at,
        providers: members.iter().map(|m| member_response(m, &providers, &configs)).collect(),
    })
}

async fn reload(state: &AppState) -> Result<(), TenantError> {
    crate::api::providers::reload(state).await
}

async fn owned_member(scope: &Scope, state: &AppState, id: i64) -> Result<RoutingConfigProvider, TenantError> {
    let member = state
        .db
        .get_routing_config_provider(id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Routing config provider not found"))?;
    scope.config(state, member.routing_config_id).await.map_err(|_| not_found("Routing config provider not found"))?;
    Ok(member)
}

pub async fn list_routing_configs(State(state): State<Arc<AppState>>, user: UserExtractor) -> ApiResult<Vec<RoutingConfigFullResponse>> {
    let configs = Scope::from(user).configs(&state).await?;
    let mut out = Vec::with_capacity(configs.len());
    for config in configs {
        out.push(full_response(&state, config).await?);
    }
    Ok(Json(out))
}

pub async fn create_routing_config(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(request): Json<RoutingConfigCreateRequest>,
) -> ApiResult<RoutingConfigFullResponse> {
    let scope = Scope::from(user);
    validate_health_seconds(Some(request.health_check_interval_seconds), Some(request.health_check_timeout_seconds))?;
    let declared = RequestedModalities::parse(request.input_modalities.as_deref(), request.output_modalities.as_deref())?
        .apply(Default::default());
    let config = crate::db::NewRoutingConfig {
        name: scope.config_name(&request.name)?,
        strategy: request.strategy.clone(),
        health_check_enabled: request.health_check_enabled,
        health_check_interval_seconds: request.health_check_interval_seconds,
        health_check_timeout_seconds: request.health_check_timeout_seconds,
    };
    let created = state.db.create_owned_routing_config(scope.owner(), config).await.map_err(internal)?;
    let created = store_modalities(&state, created, &declared).await?;
    Ok(Json(full_response(&state, created).await?))
}

pub async fn update_routing_config(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(request): Json<RoutingConfigUpdateRequest>,
) -> ApiResult<RoutingConfigFullResponse> {
    let scope = Scope::from(user);
    scope.config(&state, id).await?;
    validate_health_seconds(request.health_check_interval_seconds, request.health_check_timeout_seconds)?;
    let requested = RequestedModalities::parse(request.input_modalities.as_deref(), request.output_modalities.as_deref())?;
    let name = match request.name.as_deref() {
        Some(name) => Some(scope.config_name(name)?),
        None => None,
    };
    let updates = crate::db::UpdateRoutingConfig {
        name,
        strategy: request.strategy.clone(),
        health_check_enabled: request.health_check_enabled,
        health_check_interval_seconds: request.health_check_interval_seconds,
        health_check_timeout_seconds: request.health_check_timeout_seconds,
    };
    let updated = state.db.update_routing_config(id, updates).await.map_err(internal)?;
    let declared = requested.apply(updated.declared_modalities());
    let updated = store_modalities(&state, updated, &declared).await?;
    reload(&state).await?;
    Ok(Json(full_response(&state, updated).await?))
}

pub async fn delete_routing_config(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
) -> ApiResult<Message> {
    Scope::from(user).config(&state, id).await?;
    if !state.db.delete_routing_config(id).await.map_err(internal)? {
        return Err(not_found("Routing config not found"));
    }
    reload(&state).await?;
    Ok(Json(Message { message: "Routing config deleted successfully", id: Some(id) }))
}

pub async fn create_routing_config_provider(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(request): Json<RoutingConfigProviderCreateRequest>,
) -> ApiResult<Message> {
    let scope = Scope::from(user);
    let parent = scope.config(&state, request.routing_config_id).await?;
    let extra_body = crate::api::shared::parse_extra_body(request.extra_body.clone())?;
    let created = match (request.provider_id, request.config_id) {
        (Some(provider_id), None) => {
            let provider = scope.provider_by_id(&state, provider_id).await?;
            if provider.owner_user_id.is_some() && provider.owner_user_id != parent.owner_user_id {
                if !(scope.is_admin() && parent.owner_user_id.is_none()) {
                    return Err(forbidden("A user's provider can only be assigned to that user's own routing configs"));
                }
                if provider.status != crate::db::APPROVED {
                    return Err(bad_request("Only approved user providers can join a system routing config"));
                }
            }
            state
                .db
                .create_routing_config_provider(crate::db::NewRoutingConfigProvider {
                    routing_config_id: parent.id,
                    provider_id,
                    model: request.model.clone(),
                    weight: request.weight,
                    is_active: request.is_active,
                })
                .await
        }
        (None, Some(child_id)) => {
            scope.config(&state, child_id).await.map_err(|_| not_found("Child routing config not found"))?;
            state
                .db
                .validate_nested_link(parent.id, child_id)
                .await
                .map_err(internal)?
                .map_err(|e| bad_request(&e.to_string()))?;
            state
                .db
                .create_routing_config_child(crate::db::NewRoutingConfigChild {
                    routing_config_id: parent.id,
                    child_config_id: child_id,
                    model: request.model.clone(),
                    weight: request.weight,
                    is_active: request.is_active,
                })
                .await
        }
        _ => return Err(bad_request("Provide exactly one of provider_id or config_id")),
    }
    .map_err(internal)?;
    if let Some(body) = extra_body {
        state.db.set_routing_config_provider_extra_body(created.id, Some(&body)).await.map_err(internal)?;
    }
    reload(&state).await?;
    Ok(Json(Message { message: "Routing config provider created successfully", id: Some(created.id) }))
}

pub async fn update_routing_config_provider(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(request): Json<RoutingConfigProviderUpdateRequest>,
) -> ApiResult<Message> {
    let scope = Scope::from(user);
    owned_member(&scope, &state, id).await?;
    let extra_body = crate::api::shared::parse_extra_body(request.extra_body.clone())?;
    let updates = crate::db::UpdateRoutingConfigProvider {
        model: request.model.clone(),
        weight: request.weight,
        is_active: request.is_active,
    };
    let updated = state.db.update_routing_config_provider(id, updates).await.map_err(internal)?;
    if request.extra_body.is_some() {
        state.db.set_routing_config_provider_extra_body(updated.id, extra_body.as_ref()).await.map_err(internal)?;
    }
    reload(&state).await?;
    Ok(Json(Message { message: "Routing config provider updated successfully", id: Some(id) }))
}

pub async fn delete_routing_config_provider(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
) -> ApiResult<Message> {
    let scope = Scope::from(user);
    owned_member(&scope, &state, id).await?;
    if !state.db.delete_routing_config_provider(id).await.map_err(internal)? {
        return Err(not_found("Routing config provider not found"));
    }
    reload(&state).await?;
    Ok(Json(Message { message: "Routing config provider deleted successfully", id: Some(id) }))
}
