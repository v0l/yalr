use crate::state::AppState;
use axum::{
    extract::{Query, State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::IntoResponse,
};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct WsMetricsQuery {
    pub token: Option<String>,
}

/// Admin-only WebSocket endpoint that streams real-time metrics events.
///
/// Connect via: `ws://host/api/metrics/ws?token=<bearer_token>`
///
/// Each message is a JSON-encoded `ProviderMetrics` event pushed in real time
/// as they are emitted by the metrics system.
pub async fn ws_metrics_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsMetricsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // Authenticate before upgrading
    if !is_admin(&state, params.token.as_deref()).await {
        return (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn is_admin(state: &Arc<AppState>, token: Option<&str>) -> bool {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };

    // Try session store first
    if let Some(session) = state.session_store.validate(token).await {
        if session.is_admin {
            return true;
        }
    }

    // Try API key
    if let Some(user) = validate_api_key(state, token).await {
        if user.is_admin {
            return true;
        }
    }

    false
}

async fn validate_api_key(state: &Arc<AppState>, token: &str) -> Option<crate::db::User> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let key_hash = hex::encode(hasher.finalize());

    if let Ok(Some(api_key)) = state.db.get_api_key_by_hash(&key_hash).await {
        if api_key.is_active {
            let is_expired = state.db.is_api_key_expired(api_key.id).await.unwrap_or(false);
            if !is_expired {
                if let Ok(Some(user)) = state.db.get_user_by_id(api_key.user_id).await {
                    return Some(user);
                }
            }
        }
    }

    None
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let mut receiver = state.metrics_emitter.receiver();

    loop {
        tokio::select! {
            result = receiver.recv() => {
                match result {
                    Ok(metrics) => {
                        let json = match serde_json::to_string(&metrics) {
                            Ok(j) => j,
                            Err(e) => {
                                tracing::error!("Failed to serialize metrics event: {}", e);
                                continue;
                            }
                        };
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            // Client disconnected
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "Metrics WS client lagged behind");
                        // Send a lag notification so the client knows it missed events
                        let msg = serde_json::json!({"type": "lag", "skipped": n});
                        let _ = socket.send(Message::Text(msg.to_string().into())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("Metrics broadcast channel closed, ending WS connection");
                        break;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::admin::SessionStore;
    use crate::db::{Database, NewUser, UserType};
    use crate::metrics::MetricsStore;
    use std::sync::Arc;

    async fn setup_test_state() -> (Arc<AppState>, MetricsStore) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let metrics_store = MetricsStore::new(1000);

        let app_config = crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router: Arc::new(crate::router::engine::Router::new(
                Arc::new(crate::router::strategies::round_robin::RoundRobinStrategy::new()),
                metrics_store.clone(),
                Arc::new(db.clone()),
            )),
            auth_config: crate::auth::nip98::AuthConfig::default(),
        };

        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: app_config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.clone(),
            session_store,
            db: Arc::new(db),
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
    async fn test_ws_handler_receives_metrics_events() {
        let (state, metrics_store) = setup_test_state().await;
        let emitter = metrics_store.emitter().clone();

        // Simulate what handle_ws does: subscribe and receive
        let mut receiver = state.metrics_emitter.receiver();

        // Emit an event
        emitter.emit_ttft("test-provider", "test-model", 42);

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

        emitter.emit_success("test-provider", "test-model");

        let event = receiver.recv().await.unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["provider"], "test-provider");
        assert_eq!(parsed["model"], "test-model");
        assert_eq!(parsed["event"], "Success");
    }
}
