use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::auth::admin::UserExtractor;
use crate::db::{NewProvider, Provider, ProviderType, UpdateProvider};
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Serialize)]
pub struct MyProvider {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub provider_type: String,
    pub has_api_key: bool,
    pub owner_user_id: Option<i64>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
}

impl From<Provider> for MyProvider {
    fn from(p: Provider) -> Self {
        Self {
            id: p.id,
            name: p.name,
            slug: p.slug,
            base_url: p.base_url,
            provider_type: p.provider_type.as_str().to_string(),
            has_api_key: p.api_key.as_deref().is_some_and(|k| !k.is_empty()),
            owner_user_id: p.owner_user_id,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateMyProvider {
    pub name: String,
    pub slug: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub provider_type: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateMyProvider {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub provider_type: Option<String>,
}

#[derive(Serialize)]
pub struct MyProviderHealth {
    pub healthy: bool,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct MyProviderModels {
    pub models: Vec<String>,
}

fn internal(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn user_type(raw: Option<&str>) -> Result<ProviderType, (StatusCode, String)> {
    let kind = raw.and_then(ProviderType::from_str).unwrap_or(ProviderType::OpenAi);
    if kind.is_oauth() {
        return Err((StatusCode::BAD_REQUEST, "OAuth providers cannot be user owned".to_string()));
    }
    Ok(kind)
}

fn namespaced_slug(user_id: i64, slug: &str) -> Result<String, (StatusCode, String)> {
    let clean: String = slug
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if clean.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "slug is required".to_string()));
    }
    Ok(format!("u{user_id}-{clean}"))
}

async fn guard_url(base_url: &str) -> Result<(), (StatusCode, String)> {
    crate::ssrf::check_public_url(base_url)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

pub async fn owned_provider(
    state: &AppState,
    user: &crate::db::User,
    slug: &str,
) -> Result<Provider, (StatusCode, String)> {
    let provider = state
        .db
        .get_provider_by_slug(slug)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "Provider not found".to_string()))?;
    if provider.owner_user_id == Some(user.id) || user.is_admin {
        Ok(provider)
    } else {
        Err((StatusCode::NOT_FOUND, "Provider not found".to_string()))
    }
}

async fn reload(state: &AppState) -> Result<(), (StatusCode, String)> {
    state.config.router.reload_config().await.map_err(|e| internal(e))
}

pub async fn list(State(state): State<Arc<AppState>>, UserExtractor(auth): UserExtractor) -> ApiResult<Vec<MyProvider>> {
    let rows = state.db.list_providers_owned_by(auth.user.id).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(MyProvider::from).collect()))
}

pub async fn create(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Json(req): Json<CreateMyProvider>,
) -> ApiResult<MyProvider> {
    guard_url(&req.base_url).await?;
    let kind = user_type(req.provider_type.as_deref())?;
    let slug = namespaced_slug(auth.user.id, &req.slug)?;
    let created = state
        .db
        .create_owned_provider(
            auth.user.id,
            NewProvider {
                name: &format!("{}/{}", auth.user.username.as_deref().unwrap_or("user"), req.name.trim()),
                slug: &slug,
                base_url: req.base_url.trim(),
                api_key: req.api_key.as_deref().filter(|k| !k.is_empty()),
                provider_type: Some(kind),
            },
        )
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db) if db.is_unique_violation() => {
                (StatusCode::CONFLICT, "A provider with that name or slug exists".to_string())
            }
            other => internal(other),
        })?;
    reload(&state).await?;
    Ok(Json(created.into()))
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(slug): Path<String>,
    Json(req): Json<UpdateMyProvider>,
) -> ApiResult<MyProvider> {
    let provider = owned_provider(&state, &auth.user, &slug).await?;
    if let Some(url) = req.base_url.as_deref() {
        guard_url(url).await?;
    }
    let kind = match req.provider_type.as_deref() {
        Some(raw) => Some(user_type(Some(raw))?),
        None => None,
    };
    let updated = state
        .db
        .update_provider(
            provider.id,
            UpdateProvider {
                name: req.name.as_deref(),
                slug: None,
                base_url: req.base_url.as_deref(),
                api_key: req.api_key.as_deref().filter(|k| !k.is_empty()).map(Some),
                provider_type: kind,
            },
        )
        .await
        .map_err(internal)?;
    reload(&state).await?;
    Ok(Json(updated.into()))
}

pub async fn delete(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(slug): Path<String>,
) -> ApiResult<serde_json::Value> {
    let provider = owned_provider(&state, &auth.user, &slug).await?;
    state.db.delete_provider(provider.id).await.map_err(internal)?;
    reload(&state).await?;
    Ok(Json(serde_json::Value::Bool(true)))
}

async fn live(state: &AppState, slug: &str) -> Result<Arc<dyn crate::providers::Provider>, (StatusCode, String)> {
    state
        .config
        .router
        .get_providers()
        .await
        .into_iter()
        .find(|p| p.slug() == slug)
        .ok_or((StatusCode::BAD_GATEWAY, "Provider is not loaded".to_string()))
}

pub async fn health(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(slug): Path<String>,
) -> ApiResult<MyProviderHealth> {
    owned_provider(&state, &auth.user, &slug).await?;
    let provider = live(&state, &slug).await?;
    let result = provider.health_check().await;
    Ok(Json(MyProviderHealth {
        healthy: matches!(result, Ok(true)),
        error: result.err().map(|e| e.to_string()),
    }))
}

pub async fn models(
    State(state): State<Arc<AppState>>,
    UserExtractor(auth): UserExtractor,
    Path(slug): Path<String>,
) -> ApiResult<MyProviderModels> {
    owned_provider(&state, &auth.user, &slug).await?;
    let provider = live(&state, &slug).await?;
    let models = provider.list_models().await.map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(MyProviderModels { models: models.into_iter().map(|m| m.id).collect() }))
}
