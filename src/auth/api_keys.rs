use std::sync::Arc;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use chrono::{Duration, Utc};
use crate::auth::api_key::{generate_api_key, get_last_four, hash_api_key};
use crate::auth::admin::{UserExtractor, AdminExtractor};
use crate::state::AppState;
use crate::db::NewApiKey;

pub async fn create_api_key(
    State(state): State<Arc<AppState>>,
    UserExtractor(user): UserExtractor,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyResponse>, (StatusCode, String)> {
    let user_id = user.id;
    
    let plain_key = generate_api_key();
    let key_hash = hash_api_key(&plain_key);
    let last_four = get_last_four(&plain_key);
    
    let expires_at = req.expires_in_days.and_then(|days| {
        if days > 0 {
            Some(Utc::now() + Duration::days(days))
        } else {
            None
        }
    });

    let api_key = state.db.create_api_key(NewApiKey {
        key_hash: &key_hash,
        name: &req.name,
        user_id,
        last_four: &last_four,
        expires_at: expires_at.map(|dt| dt.naive_utc()),
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyResponse {
        id: api_key.id,
        name: api_key.name,
        key: plain_key,
        last_four: api_key.last_four,
        created_at: api_key.created_at,
        expires_at: expires_at.map(|dt| dt.to_string()),
    }))
}

pub async fn create_api_key_for_user(
    State(state): State<Arc<AppState>>,
    Path(user_id): Path<i64>,
    AdminExtractor(_admin): AdminExtractor, // Verify admin access
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyResponse>, (StatusCode, String)> {
    // Admin is creating a key for another user
    let plain_key = generate_api_key();
    let key_hash = hash_api_key(&plain_key);
    let last_four = get_last_four(&plain_key);
    
    let expires_at = req.expires_in_days.and_then(|days| {
        if days > 0 {
            Some(Utc::now() + Duration::days(days))
        } else {
            None
        }
    });

    let api_key = state.db.create_api_key(NewApiKey {
        key_hash: &key_hash,
        name: &req.name,
        user_id,
        last_four: &last_four,
        expires_at: expires_at.map(|dt| dt.naive_utc()),
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyResponse {
        id: api_key.id,
        name: api_key.name,
        key: plain_key,
        last_four: api_key.last_four,
        created_at: api_key.created_at,
        expires_at: expires_at.map(|dt| dt.to_string()),
    }))
}

pub async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    UserExtractor(user): UserExtractor,
) -> Result<Json<Vec<ApiKeyListItem>>, (StatusCode, String)> {
    let keys = state.db.list_api_keys_for_user(user.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<ApiKeyListItem> = keys.into_iter().map(|k| {
        ApiKeyListItem {
            id: k.id,
            name: k.name,
            last_four: k.last_four,
            created_at: k.created_at,
            expires_at: k.expires_at,
            is_active: k.is_active,
        }
    }).collect();

    Ok(Json(items))
}

#[derive(serde::Serialize)]
pub struct ApiKeyResponse {
    pub id: i64,
    pub name: String,
    pub key: String, // Only shown once at creation
    pub last_four: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ApiKeyListItem {
    pub id: i64,
    pub name: String,
    pub last_four: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub is_active: bool,
}

#[derive(serde::Serialize)]
pub struct ApiKeyDeleteResponse {
    pub deleted: bool,
    pub id: i64,
}

#[derive(serde::Serialize)]
pub struct ApiKeyDisableResponse {
    pub disabled: bool,
    pub id: i64,
}

#[derive(serde::Serialize)]
pub struct ApiKeyEnableResponse {
    pub enabled: bool,
    pub id: i64,
}

#[derive(serde::Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub expires_in_days: Option<i64>,
}

pub async fn delete_api_key(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiKeyDeleteResponse>, (StatusCode, String)> {
    state.db.delete_api_key(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyDeleteResponse {
        deleted: true,
        id,
    }))
}

pub async fn disable_api_key(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiKeyDisableResponse>, (StatusCode, String)> {
    state.db.disable_api_key(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyDisableResponse {
        disabled: true,
        id,
    }))
}

pub async fn enable_api_key(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiKeyEnableResponse>, (StatusCode, String)> {
    state.db.enable_api_key(id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyEnableResponse {
        enabled: true,
        id,
    }))
}
