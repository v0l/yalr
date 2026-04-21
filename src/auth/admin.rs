use std::sync::Arc;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use crate::state::AppState;
use crate::db::UserType;

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
    let users = db.list_users()
        .await
        .unwrap_or_default();
    
    Json(SetupStatusResponse {
        setup_complete: !users.is_empty(),
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
    
    let users = db.list_users().await.unwrap_or_default();
    if !users.is_empty() {
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
            // NIP-98 auth - extract pubkey and get/create user
            use nostr::{Event, JsonUtil, Kind};
            use base64::Engine;
            use base64::prelude::BASE64_STANDARD;
            
            let auth_str = auth.strip_prefix("Nostr ").unwrap();
            let event_bytes = BASE64_STANDARD.decode(auth_str).map_err(|_| StatusCode::UNAUTHORIZED)?;
            let event = Event::from_json(event_bytes).map_err(|_| StatusCode::UNAUTHORIZED)?;

            if event.kind != Kind::HttpAuth || event.verify().is_err() {
                return Err(StatusCode::UNAUTHORIZED);
            }

            let pubkey = event.pubkey.to_string();
            let user = state.db.get_or_create_user_by_nostr_pubkey(&pubkey)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            
            // Attach user to request extensions
            req.extensions_mut().insert(user);
            return Ok(next.run(req).await);
        } else if auth.starts_with("Bearer ") {
            // Bearer token auth
            let session_store = &state.session_store;
            let session_id = auth.strip_prefix("Bearer ");
            
            if let Some(token) = session_id {
                if session_store.validate(token).await.is_some() {
                    return Ok(next.run(req).await);
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}
