use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Deserialize;

use crate::auth::admin::LoginResponse;
use crate::db::{NewUser, UserType};
use crate::state::AppState;

const WINDOW: Duration = Duration::from_secs(60 * 60);
const MAX_PER_WINDOW: usize = 10;
const SESSION_SECS: u64 = 24 * 60 * 60;
const MIN_PASSWORD: usize = 8;
const MAX_PASSWORD: usize = 256;

type Failure = (StatusCode, String);

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

#[derive(Default)]
struct Limiter {
    hits: Mutex<HashMap<String, Vec<Instant>>>,
}

impl Limiter {
    fn allow(&self, ip: &str) -> bool {
        let mut hits = self.hits.lock().unwrap();
        hits.retain(|_, times| times.iter().any(|t| t.elapsed() < WINDOW));
        let times = hits.entry(ip.to_string()).or_default();
        times.retain(|t| t.elapsed() < WINDOW);
        if times.len() >= MAX_PER_WINDOW {
            return false;
        }
        times.push(Instant::now());
        true
    }
}

fn limiter() -> &'static Limiter {
    static LIMITER: OnceLock<Limiter> = OnceLock::new();
    LIMITER.get_or_init(Limiter::default)
}

fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .or_else(|| peer.map(|p| p.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn valid_username(name: &str) -> bool {
    (3..=32).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !name.starts_with("nostr_")
        && !name.starts_with("anon_")
}

fn hash_password(password: &str) -> Result<String, Failure> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    argon2::Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn internal(e: impl ToString) -> Failure {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn registration_closed() -> Failure {
    (StatusCode::FORBIDDEN, "Registration is closed".to_string())
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<LoginResponse>, Failure> {
    if !state.config.auth_config.open_registration {
        return Err(registration_closed());
    }
    let username = req.username.trim().to_string();
    if !valid_username(&username) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Username must be 3-32 characters: letters, digits, dot, dash or underscore".to_string(),
        ));
    }
    if req.password.len() < MIN_PASSWORD || req.password.len() > MAX_PASSWORD {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Password must be {MIN_PASSWORD}-{MAX_PASSWORD} characters"),
        ));
    }
    if !limiter().allow(&client_ip(&headers, peer.map(|p| p.0))) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "Too many sign ups from this address, try later".to_string()));
    }
    if state.db.user_exists(&username).await.map_err(internal)? {
        return Err((StatusCode::CONFLICT, "Username is taken".to_string()));
    }
    let hash = hash_password(&req.password)?;
    let user = state
        .db
        .create_user(NewUser {
            username: Some(&username),
            password_hash: Some(&hash),
            external_id: None,
            user_type: UserType::Internal,
            is_admin: false,
        })
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref db) if db.is_unique_violation() => {
                (StatusCode::CONFLICT, "Username is taken".to_string())
            }
            other => internal(other),
        })?;
    let token = state.session_store.create(&username, false, SESSION_SECS).await;
    Ok(Json(LoginResponse { token, display_name: user.shown_name(), username, is_admin: user.is_admin }))
}

pub async fn nostr_login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<LoginResponse>, Failure> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "Missing Nostr authorization".to_string()))?;
    if !state.config.auth_config.enabled {
        return Err((StatusCode::FORBIDDEN, "Nostr sign in is disabled on this server".to_string()));
    }
    let pubkey = crate::auth::admin::verified_pubkey(&state, header, "POST", "/api/auth/nostr")
        .map_err(|code| (code, "Invalid Nostr authorization".to_string()))?;
    let existing = state.db.get_user_by_external_id(&pubkey, UserType::Nostr).await.map_err(internal)?;
    if existing.is_none() && !state.config.auth_config.open_registration {
        return Err(registration_closed());
    }
    let user = state.db.get_or_create_user_by_nostr_pubkey(&pubkey).await.map_err(internal)?;
    crate::auth::admin::refresh_profile(&state, &user);
    let username = user.username.clone().unwrap_or_default();
    let token = state.session_store.create(&username, user.is_admin, SESSION_SECS).await;
    Ok(Json(LoginResponse { token, username, display_name: user.shown_name(), is_admin: user.is_admin }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::admin::SessionStore;
    use crate::db::Database;
    use crate::metrics::MetricsStore;
    use axum::body::Body;
    use axum::http::Request;
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    use tower::util::ServiceExt;

    async fn app(open: bool) -> (Arc<AppState>, axum::Router) {
        let db = Arc::new(Database::test().await);
        let metrics_store = Arc::new(MetricsStore::new(100));
        let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
        let mut auth_config = crate::auth::nip98::AuthConfig::default();
        auth_config.enabled = true;
        auth_config.open_registration = open;
        let state = Arc::new(AppState {
            config: crate::config::AppConfig {
                db: db.clone(),
                router,
                auth_config,
                payments_config: None,
                discovery_config: None,
                admin_ui_path: String::new(),
                host: String::new(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(SessionStore::new()),
            db,
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
        public_cache: Default::default(),
        });
        let app = crate::api::server::create_test_app(state.clone()).await;
        (state, app)
    }

    fn register_request(ip: &str, username: &str, password: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header("content-type", "application/json")
            .header("x-forwarded-for", ip)
            .body(Body::from(format!(r#"{{"username":"{username}","password":"{password}"}}"#)))
            .unwrap()
    }

    fn nostr_request(keys: &Keys) -> Request<Body> {
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(vec![
                Tag::parse(["u", "https://yalr.example/api/auth/nostr"]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        Request::builder()
            .method("POST")
            .uri("/api/auth/nostr")
            .header("authorization", format!("Nostr {}", BASE64_STANDARD.encode(event.as_json())))
            .body(Body::empty())
            .unwrap()
    }

    async fn body_json(res: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    #[tokio::test]
    async fn password_sign_up_creates_a_user_who_can_log_in() {
        let (state, app) = app(true).await;
        let res = app.clone().oneshot(register_request("1.1.1.1", "newbie", "correct horse")).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        assert_eq!(body["username"], "newbie");
        assert_eq!(body["is_admin"], false);
        assert!(state.session_store.validate(body["token"].as_str().unwrap()).await.is_some());
        let stored = state.db.get_user_by_username("newbie").await.unwrap().unwrap();
        assert!(stored.password_hash.unwrap().starts_with("$argon2"));

        let login = Request::builder()
            .method("POST")
            .uri("/api/auth/login")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"username":"newbie","password":"correct horse"}"#))
            .unwrap();
        assert_eq!(app.oneshot(login).await.unwrap().status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn sign_up_rejects_duplicate_and_weak_input() {
        let (_state, app) = app(true).await;
        assert_eq!(
            app.clone().oneshot(register_request("2.2.2.2", "dupe", "password1")).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(
            app.clone().oneshot(register_request("2.2.2.2", "dupe", "password2")).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            app.clone().oneshot(register_request("2.2.2.2", "ok-name", "short")).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.clone().oneshot(register_request("2.2.2.2", "nostr_abc", "password1")).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            app.oneshot(register_request("2.2.2.2", "a b", "password1")).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn sign_up_is_rate_limited_per_ip() {
        let (_state, app) = app(true).await;
        for i in 0..MAX_PER_WINDOW {
            let res = app.clone().oneshot(register_request("3.3.3.3", &format!("user{i}"), "password1")).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{i}");
        }
        let res = app.clone().oneshot(register_request("3.3.3.3", "one-too-many", "password1")).await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        let other = app.oneshot(register_request("3.3.3.4", "elsewhere", "password1")).await.unwrap();
        assert_eq!(other.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn closed_registration_refuses_new_users_but_not_existing_logins() {
        let (state, app) = app(false).await;
        let res = app.clone().oneshot(register_request("4.4.4.4", "late", "password1")).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert!(state.db.get_user_by_username("late").await.unwrap().is_none());

        let keys = Keys::generate();
        let res = app.clone().oneshot(nostr_request(&keys)).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        state.db.get_or_create_user_by_nostr_pubkey(&keys.public_key().to_string()).await.unwrap();
        let res = app.oneshot(nostr_request(&keys)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn nostr_sign_in_creates_the_user_and_returns_a_working_session() {
        let (state, app) = app(true).await;
        let keys = Keys::generate();
        let res = app.clone().oneshot(nostr_request(&keys)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = body_json(res).await;
        let pubkey = keys.public_key().to_string();
        assert_eq!(body["username"], format!("nostr_{}", &pubkey[..16]));
        assert_eq!(body["is_admin"], false);
        let token = body["token"].as_str().unwrap().to_string();
        assert!(state.db.get_user_by_external_id(&pubkey, UserType::Nostr).await.unwrap().is_some());

        let again = body_json(app.clone().oneshot(nostr_request(&keys)).await.unwrap()).await;
        assert_eq!(again["username"], body["username"]);

        let status = Request::builder()
            .uri("/api/auth/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let status = body_json(app.oneshot(status).await.unwrap()).await;
        assert_eq!(status["authenticated"], true);
    }

    #[tokio::test]
    async fn nostr_sign_in_reports_disabled_when_nostr_auth_is_off() {
        let (state, _) = app(true).await;
        let mut off = (*state).clone();
        off.config.auth_config.enabled = false;
        let app = crate::api::server::create_test_app(Arc::new(off)).await;

        let status = Request::builder().uri("/api/setup/status").body(Body::empty()).unwrap();
        let status = body_json(app.clone().oneshot(status).await.unwrap()).await;
        assert_eq!(status["nostr_login"], false);

        let res = app.oneshot(nostr_request(&Keys::generate())).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let text = String::from_utf8(axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert_eq!(text, "Nostr sign in is disabled on this server");
    }

    #[tokio::test]
    async fn setup_status_reports_nostr_login_when_enabled() {
        let (_state, app) = app(true).await;
        let status = Request::builder().uri("/api/setup/status").body(Body::empty()).unwrap();
        let status = body_json(app.oneshot(status).await.unwrap()).await;
        assert_eq!(status["nostr_login"], true);
    }

    #[tokio::test]
    async fn nostr_sign_in_rejects_a_bad_signature_target() {
        let (_state, app) = app(true).await;
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(vec![
                Tag::parse(["u", "https://yalr.example/api/other"]).unwrap(),
                Tag::parse(["method", "POST"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/nostr")
            .header("authorization", format!("Nostr {}", BASE64_STANDARD.encode(event.as_json())))
            .body(Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    }
}
