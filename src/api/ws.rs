use crate::state::AppState;
use axum::{
    extract::{Query, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const VISIBILITY_TTL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
pub struct WsMetricsQuery {
    pub token: Option<String>,
}

pub async fn ws_metrics_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsMetricsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let Some(user) = token_user(&state, params.token.as_deref()).await else {
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    ws.on_upgrade(move |socket| handle_ws(socket, state, user))
}

async fn token_user(state: &Arc<AppState>, token: Option<&str>) -> Option<crate::db::User> {
    let token = token.filter(|t| !t.is_empty())?;
    if let Some(session) = state.session_store.validate(token).await {
        if let Ok(Some(user)) = state.db.get_user_by_username(&session.username).await {
            return Some(user);
        }
    }
    validate_api_key(state, token).await
}

#[cfg(test)]
async fn is_admin(state: &Arc<AppState>, token: Option<&str>) -> bool {
    token_user(state, token).await.is_some_and(|u| u.is_admin)
}

async fn validate_api_key(state: &Arc<AppState>, token: &str) -> Option<crate::db::User> {
    use sha2::{Digest, Sha256};

    let key_hash = hex::encode(Sha256::digest(token.as_bytes()));
    let api_key = state.db.get_api_key_by_hash(&key_hash).await.ok().flatten().filter(|k| k.is_active)?;
    if state.db.is_api_key_expired(api_key.id).await.unwrap_or(false) {
        return None;
    }
    state.db.get_user_by_id(api_key.user_id).await.ok().flatten()
}

struct Visibility {
    user: crate::db::User,
    names: Option<HashSet<String>>,
    refreshed: Instant,
}

impl Visibility {
    async fn new(state: &AppState, user: crate::db::User) -> Self {
        let mut visibility = Self { user, names: None, refreshed: Instant::now() };
        visibility.refresh(state).await;
        visibility
    }

    async fn refresh(&mut self, state: &AppState) {
        let scope = crate::api::tenant::Scope { user: self.user.clone() };
        self.names = scope.provider_names(state).await.ok().flatten().map(|n| n.into_iter().collect());
        if !self.user.is_admin && self.names.is_none() {
            self.names = Some(HashSet::new());
        }
        self.refreshed = Instant::now();
    }

    async fn admits(&mut self, state: &AppState, provider: &str) -> bool {
        if self.refreshed.elapsed() > VISIBILITY_TTL {
            self.refresh(state).await;
        }
        self.names.as_ref().is_none_or(|names| names.contains(provider))
    }
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>, user: crate::db::User) {
    let mut receiver = state.metrics_emitter.receiver();
    let mut visibility = Visibility::new(&state, user).await;

    loop {
        tokio::select! {
            result = receiver.recv() => {
                match result {
                    Ok(metrics) => {
                        if !visibility.admits(&state, &metrics.provider).await {
                            continue;
                        }
                        let json = match serde_json::to_string(&metrics) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize metrics event: {}", e);
                                continue;
                            }
                        };
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "Metrics WS client lagged behind");
                        let lag = Lag { kind: "lag", skipped: n };
                        let _ = socket.send(Message::Text(serde_json::to_string(&lag).unwrap_or_default())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("Metrics broadcast channel closed, ending WS connection");
                        break;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(serde::Serialize)]
struct Lag {
    #[serde(rename = "type")]
    kind: &'static str,
    skipped: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::admin::SessionStore;
    use crate::db::{Database, NewUser, UserType};
    use crate::metrics::MetricsStore;
    use std::sync::Arc;

    async fn setup_test_state() -> (Arc<AppState>, MetricsStore) {
        let db = Database::test().await;
        let metrics_store = MetricsStore::new(1000);

        let app_config = crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router: Arc::new(crate::router::engine::Router::new(
                metrics_store.clone(),
                Arc::new(db.clone()),
            )),
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
            discovery_config: None,
            admin_ui_path: "/app/admin/dist".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        };

        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: app_config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.clone().into(),
            session_store,
            db: Arc::new(db),
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
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

    async fn setup_non_admin_user(state: &Arc<AppState>) -> String {
        use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
        use rand::rngs::OsRng;

        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let password_hash = argon2
            .hash_password(b"password123", &salt)
            .unwrap()
            .to_string();

        state.db.create_user(NewUser {
            username: Some("viewer"),
            password_hash: Some(&password_hash),
            external_id: None,
            user_type: UserType::Internal,
            is_admin: false,
        }).await.unwrap();

        state.session_store.create("viewer", false, 86400).await
    }

    #[tokio::test]
    async fn test_is_admin_rejects_no_token() {
        let (state, _) = setup_test_state().await;
        assert!(!is_admin(&state, None).await);
    }

    #[tokio::test]
    async fn test_is_admin_rejects_empty_token() {
        let (state, _) = setup_test_state().await;
        assert!(!is_admin(&state, Some("")).await);
    }

    #[tokio::test]
    async fn test_is_admin_rejects_invalid_token() {
        let (state, _) = setup_test_state().await;
        assert!(!is_admin(&state, Some("invalid-token")).await);
    }

    #[tokio::test]
    async fn test_is_admin_accepts_admin_session_token() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        assert!(is_admin(&state, Some(&token)).await);
    }

    #[tokio::test]
    async fn test_is_admin_rejects_non_admin_session_token() {
        let (state, _) = setup_test_state().await;
        let token = setup_non_admin_user(&state).await;
        assert!(!is_admin(&state, Some(&token)).await);
    }

    #[tokio::test]
    async fn a_tenant_stream_carries_only_its_own_providers_events() {
        let (state, _) = setup_test_state().await;
        let viewer = state.db.add_test_user("tenant", false).await;
        state
            .db
            .create_owned_provider(
                viewer.id,
                crate::db::NewProvider {
                    name: "tenant/gpu",
                    slug: "u-tenant-gpu",
                    base_url: "http://127.0.0.1:9/v1",
                    api_key: None,
                    provider_type: None,
                },
            )
            .await
            .unwrap();
        let mut visibility = Visibility::new(&state, viewer).await;
        assert!(visibility.admits(&state, "tenant/gpu").await);
        assert!(!visibility.admits(&state, "platform-openai").await);

        let admin = state.db.add_test_user("root", true).await;
        let mut everything = Visibility::new(&state, admin).await;
        assert!(everything.admits(&state, "platform-openai").await);
    }

    #[tokio::test]
    async fn test_ws_handler_receives_metrics_events() {
        let (state, metrics_store) = setup_test_state().await;
        let emitter = metrics_store.emitter().clone();

        // Simulate what handle_ws does: subscribe and receive
        let mut receiver = state.metrics_emitter.receiver();

        // Emit an event
        emitter.emit_ttft("test-provider", "test-model", 42, None);

        // Receive it
        let event = receiver.recv().await.unwrap();
        assert_eq!(event.provider, "test-provider");
        assert_eq!(event.model, "test-model");
        match event.event {
            crate::metrics::MetricsEvent::TTFT(ms) => assert_eq!(ms, 42),
            _ => panic!("Expected TTFT event"),
        }
    }

    #[tokio::test]
    async fn test_ws_handler_serializes_events_as_json() {
        let (state, metrics_store) = setup_test_state().await;
        let emitter = metrics_store.emitter().clone();

        let mut receiver = state.metrics_emitter.receiver();

        emitter.emit_success("test-provider", "test-model", None);

        let event = receiver.recv().await.unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["provider"], "test-provider");
        assert_eq!(parsed["model"], "test-model");
        assert_eq!(parsed["event"], "Success");
    }
}
