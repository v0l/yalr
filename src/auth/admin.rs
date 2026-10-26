use std::sync::Arc;
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use crate::state::AppState;
use crate::db::{User, UserType, ApiKey};
use async_trait::async_trait;

/// Authenticated user context with optional API key information
#[derive(Clone)]
pub struct AuthenticatedUser {
    pub user: User,
    /// API key used for authentication (only set when authenticating via API key)
    pub api_key: Option<ApiKey>,
}

pub struct UserExtractor(pub AuthenticatedUser);

#[async_trait]
impl<S> FromRequestParts<S> for UserExtractor
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(req: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        req.extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .map(UserExtractor)
            .ok_or((StatusCode::UNAUTHORIZED, "User not found in request".to_string()))
    }
}

pub struct AdminExtractor(pub AuthenticatedUser);

#[async_trait]
impl<S> FromRequestParts<S> for AdminExtractor
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(req: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_user = req.extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or((StatusCode::UNAUTHORIZED, "User not found in request".to_string()))?;
        
        if !auth_user.user.is_admin {
            return Err((StatusCode::FORBIDDEN, "Admin access required".to_string()));
        }
        
        Ok(AdminExtractor(auth_user))
    }
}

#[derive(Clone)]
pub struct SessionStore {
    pub sessions: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, Session>>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub username: String,
    pub is_admin: bool,
    pub created_at: u64,
    pub expires_at: u64,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn create(&self, username: &str, is_admin: bool, duration_secs: u64) -> String {
        let session_id = Uuid::new_v4().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        
        let session = Session {
            id: session_id.clone(),
            username: username.to_string(),
            is_admin,
            created_at: now,
            expires_at: now + duration_secs,
        };

        self.sessions.write().await.insert(session_id.clone(), session);
        session_id
    }

    pub async fn validate(&self, session_id: &str) -> Option<Session> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(session_id)?.clone();
        
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        if session.expires_at > now {
            Some(session)
        } else {
            None
        }
    }

    pub async fn delete(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub is_admin: bool,
}

#[derive(Serialize)]
pub struct AuthStatusResponse {
    pub authenticated: bool,
    pub username: Option<String>,
    pub is_admin: Option<bool>,
}

#[derive(Serialize)]
pub struct SetupStatusResponse {
    pub setup_complete: bool,
}

#[derive(Serialize)]
pub struct SetupUserResponse {
    pub message: String,
    pub username: String,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (StatusCode, String)> {
    let db = &state.db;
    let session_store = &state.session_store;
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    
    let user = db.get_user_by_username(&req.username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()))?;

    let parsed_hash = PasswordHash::new(user.password_hash.as_ref().unwrap())
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Invalid hash".to_string()))?;
    
    let valid = Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .is_ok();

    if !valid {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }

    let session_id = session_store.create(user.username.as_ref().unwrap(), user.is_admin, 24 * 60 * 60).await; // 24 hours

    Ok(Json(LoginResponse {
        token: session_id,
        username: user.username.unwrap(),
        is_admin: user.is_admin,
    }))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> impl IntoResponse {
    let session_store = &state.session_store;
    if let Some(cookie) = req.headers().get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').find(|s| s.trim().starts_with("session=")))
    {
        if let Some(session_id) = cookie.split('=').nth(1) {
            session_store.delete(session_id.trim()).await;
        }
    }
    
    (StatusCode::OK, "Logged out")
}

pub async fn auth_status(
    State(state): State<Arc<AppState>>,
    req: Request,
) -> Json<AuthStatusResponse> {
    let session_store = &state.session_store;
    let session_id = req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    if let Some(token) = session_id {
        if let Some(session) = session_store.validate(token).await {
            return Json(AuthStatusResponse {
                authenticated: true,
                username: Some(session.username),
                is_admin: Some(session.is_admin),
            });
        }
    }

    Json(AuthStatusResponse {
        authenticated: false,
        username: None,
        is_admin: None,
    })
}

pub async fn check_setup_complete(
    State(state): State<Arc<AppState>>,
) -> Json<SetupStatusResponse> {
    let db = &state.db;
    Json(SetupStatusResponse {
        setup_complete: db.has_users().await.unwrap_or(false),
    })
}

#[derive(Deserialize)]
pub struct SetupUserRequest {
    pub username: String,
    pub password: String,
}

pub async fn setup_first_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetupUserRequest>,
) -> Result<Json<SetupUserResponse>, (StatusCode, String)> {
    let db = &state.db;
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use rand::rngs::OsRng;
    use crate::db::NewUser;
    
    if db.has_users().await.unwrap_or(true) {
        return Err((StatusCode::BAD_REQUEST, "Setup already complete".to_string()));
    }

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(req.password.as_bytes(), &salt)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .to_string();

    db.create_user(NewUser {
        username: Some(&req.username),
        password_hash: Some(&password_hash),
        external_id: None,
        user_type: UserType::Internal,
        is_admin: true,
    }).await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(SetupUserResponse {
        message: "Admin user created successfully".to_string(),
        username: req.username,
    }))
}

fn original_path(req: &Request) -> &str {
    req.extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|u| u.0.path())
        .unwrap_or_else(|| req.uri().path())
}

async fn nostr_user(
    state: &Arc<AppState>,
    header: &str,
    method: &str,
    path: &str,
) -> Result<User, StatusCode> {
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;
    use nostr::{Event, JsonUtil};

    let encoded = header.strip_prefix("Nostr ").ok_or(StatusCode::UNAUTHORIZED)?;
    let bytes = BASE64_STANDARD.decode(encoded).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let event = Event::from_json(bytes).map_err(|_| StatusCode::UNAUTHORIZED)?;
    crate::auth::nip98::validate_event(&event, method, path).map_err(|(code, _)| code)?;

    let pubkey = event.pubkey.to_string();
    let allowed = &state.config.auth_config.allowed_pubkeys;
    if !allowed.is_empty() && !allowed.contains(&pubkey) {
        return Err(StatusCode::FORBIDDEN);
    }

    state
        .db
        .get_or_create_user_by_nostr_pubkey(&pubkey)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(auth) = &auth_header {
        if auth.starts_with("Nostr ") {
            let user = nostr_user(&state, auth, req.method().as_str(), original_path(&req)).await?;

            // Attach authenticated user to request extensions
            let authenticated_user = AuthenticatedUser {
                user,
                api_key: None,
            };
            req.extensions_mut().insert(authenticated_user);
            return Ok(next.run(req).await);
        } else if auth.starts_with("Bearer ") {
            // Bearer token auth - check session or API key
            let session_store = &state.session_store;
            let token = auth.strip_prefix("Bearer ");
            
            if let Some(token) = token {
                // First check session
                if let Some(session) = session_store.validate(token).await {
                    if let Ok(Some(user)) = state.db.get_user_by_username(&session.username).await {
                        let authenticated_user = AuthenticatedUser {
                            user,
                            api_key: None,
                        };
                        req.extensions_mut().insert(authenticated_user);
                        return Ok(next.run(req).await);
                    }
                }
                
                // Then check API key
                if let Some(authenticated_user) = validate_api_key(&state, token).await? {
                    req.extensions_mut().insert(authenticated_user);
                    return Ok(next.run(req).await);
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

pub async fn optional_auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if req.headers().contains_key(header::AUTHORIZATION) {
        return auth_middleware(State(state), req, next).await;
    }
    Ok(next.run(req).await)
}

pub async fn admin_middleware(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    if let Some(auth) = &auth_header {
        if auth.starts_with("Nostr ") {
            let user = nostr_user(&state, auth, req.method().as_str(), original_path(&req)).await?;

            // Check if user is admin
            if !user.is_admin {
                return Err(StatusCode::FORBIDDEN);
            }
            
            // Attach authenticated user to request extensions
            let authenticated_user = AuthenticatedUser {
                user,
                api_key: None,
            };
            req.extensions_mut().insert(authenticated_user);
            return Ok(next.run(req).await);
        } else if auth.starts_with("Bearer ") {
            // Bearer token auth - check session or API key
            let session_store = &state.session_store;
            let token = auth.strip_prefix("Bearer ");
            
            if let Some(token) = token {
                // First check session
                if let Some(session) = session_store.validate(token).await {
                    if session.is_admin {
                        if let Ok(Some(user)) = state.db.get_user_by_username(&session.username).await {
                            let authenticated_user = AuthenticatedUser {
                                user,
                                api_key: None,
                            };
                            req.extensions_mut().insert(authenticated_user);
                            return Ok(next.run(req).await);
                        }
                    } else {
                        return Err(StatusCode::FORBIDDEN);
                    }
                }
                
                // Then check API key (admin only)
                if let Some(authenticated_user) = validate_api_key(&state, token).await? {
                    if authenticated_user.user.is_admin {
                        req.extensions_mut().insert(authenticated_user);
                        return Ok(next.run(req).await);
                    } else {
                        return Err(StatusCode::FORBIDDEN);
                    }
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

async fn validate_api_key(
    state: &Arc<AppState>,
    token: &str,
) -> Result<Option<AuthenticatedUser>, StatusCode> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    let unavailable = |e: sqlx::Error| {
        tracing::error!(error = %e, "API key lookup failed");
        StatusCode::SERVICE_UNAVAILABLE
    };

    let found = state.db.authenticate_api_key(&key_hash).await.map_err(unavailable)?;
    Ok(found.map(|(api_key, user)| AuthenticatedUser { user, api_key: Some(api_key) }))
}
