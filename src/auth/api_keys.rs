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
    UserExtractor(authenticated_user): UserExtractor,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyResponse>, (StatusCode, String)> {
    let user_id = authenticated_user.user.id;
    
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
        expires_at,
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyResponse {
        id: api_key.id,
        name: api_key.name,
        key: plain_key,
        last_four: api_key.last_four,
        created_at: api_key.created_at,
        expires_at,
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
        expires_at,
    }).await.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ApiKeyResponse {
        id: api_key.id,
        name: api_key.name,
        key: plain_key,
        last_four: api_key.last_four,
        created_at: api_key.created_at,
        expires_at,
    }))
}

pub async fn list_api_keys(
    State(state): State<Arc<AppState>>,
    UserExtractor(authenticated_user): UserExtractor,
) -> Result<Json<Vec<ApiKeyListItem>>, (StatusCode, String)> {
    let keys = state.db.list_api_keys_for_user(authenticated_user.user.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut usage = state.db.api_key_usage_for_user(authenticated_user.user.id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<ApiKeyListItem> = keys.into_iter().map(|k| {
        ApiKeyListItem {
            usage: usage.remove(&k.id).unwrap_or_default(),
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
    pub created_at: crate::db::Timestamp,
    pub expires_at: Option<crate::db::Timestamp>,
}

#[derive(serde::Serialize)]
pub struct ApiKeyListItem {
    pub usage: crate::db::ApiKeyUsage,
    pub id: i64,
    pub name: String,
    pub last_four: String,
    pub created_at: crate::db::Timestamp,
    pub expires_at: Option<crate::db::Timestamp>,
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

pub async fn delete_my_api_key(
    Path(id): Path<i64>,
    State(state): State<Arc<AppState>>,
    UserExtractor(authenticated_user): UserExtractor,
) -> Result<Json<ApiKeyDeleteResponse>, (StatusCode, String)> {
    let internal = |e: sqlx::Error| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    let owned = state.db.list_api_keys_for_user(authenticated_user.user.id).await.map_err(internal)?;
    if !authenticated_user.user.is_admin && !owned.iter().any(|k| k.id == id) {
        return Err((StatusCode::NOT_FOUND, "API key not found".to_string()));
    }
    state.db.delete_api_key(id).await.map_err(internal)?;
    Ok(Json(ApiKeyDeleteResponse { deleted: true, id }))
}

#[cfg(test)]
mod my_key_tests {
    use super::*;
    use crate::auth::admin::{AuthenticatedUser, SessionStore};
    use crate::db::Database;
    use crate::metrics::MetricsStore;

    #[tokio::test]
    async fn a_user_manages_only_their_own_keys() {
        let db = Arc::new(Database::test().await);
        let metrics_store = Arc::new(MetricsStore::new(100));
        let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
        let state = Arc::new(AppState {
            config: crate::config::AppConfig {
                db: db.clone(),
                router,
                auth_config: Default::default(),
                payments_config: None,
                discovery_config: None,
                admin_ui_path: String::new(),
                host: String::new(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(SessionStore::new()),
            db: db.clone(),
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
        public_cache: Default::default(),
        });
        let alice = db.add_test_user("alice", false).await;
        let bob = db.add_test_user("bob", false).await;
        let who = |u: crate::db::User| UserExtractor(AuthenticatedUser { user: u, api_key: None });
        let made = create_api_key(
            State(state.clone()),
            who(alice.clone()),
            Json(CreateApiKeyRequest { name: "laptop".into(), expires_in_days: None }),
        )
        .await
        .unwrap()
        .0;
        assert!(made.key.starts_with("sk-") || !made.key.is_empty());

        let foreign = delete_my_api_key(Path(made.id), State(state.clone()), who(bob)).await;
        assert_eq!(foreign.err().unwrap().0, StatusCode::NOT_FOUND);
        assert_eq!(list_api_keys(State(state.clone()), who(alice.clone())).await.unwrap().0.len(), 1);

        delete_my_api_key(Path(made.id), State(state.clone()), who(alice.clone())).await.unwrap();
        assert!(list_api_keys(State(state), who(alice)).await.unwrap().0.is_empty());
    }
}
