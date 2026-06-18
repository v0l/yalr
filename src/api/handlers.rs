// Re-export hub — all real implementations live in sub-modules.
// Keeping this file for backwards compat so server.rs and external callers
// can still `use crate::api::handlers::*` without disruption.

pub use crate::api::chat::*;
pub use crate::api::config::*;
pub use crate::api::health::*;
pub use crate::api::model_pricing::*;
pub use crate::api::models::*;
pub use crate::api::providers::*;
pub use crate::api::responses::*;
pub use crate::api::routing::*;
pub use crate::api::users::*;

#[cfg(test)]
mod tests {
    use crate::api::server::create_test_app;
    use crate::auth::admin::SessionStore;
    use crate::config::{Config, DatabaseConfig, ServerConfig};
    use crate::db::{Database, NewUser, UserType};
    use crate::metrics::MetricsStore;
    use crate::state::AppState;
    use axum::{body::Body, http::Request};
    use serde_json::json;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    async fn setup_test_state() -> (Arc<AppState>, MetricsStore) {
        let db = Database::new("sqlite::memory:").await.unwrap();

        let metrics_store = MetricsStore::new(1000);

        let config = Config {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
                admin_ui_path: "/app/admin/dist".to_string(),
            },
            database: DatabaseConfig {
                url: "sqlite::memory:".to_string(),
            },
            auth: None,
            payments: None,
            anthropic: Default::default(),
        };

        let app_config = crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router: Arc::new(crate::router::engine::Router::new(
                metrics_store.clone(),
                Arc::new(db.clone()),
            )),
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
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

    #[tokio::test]
    async fn test_health_check() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_v1_models() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_v1_models_requires_auth() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .oneshot(Request::builder().uri("/v1/models").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn test_api_setup_status() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/api/setup/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_setup() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/setup")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "username": "admin",
                        "password": "password123"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_login() {
        let (state, _) = setup_test_state().await;
        setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/login")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "username": "admin",
                        "password": "password123"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_protected_routes_require_auth() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(Request::builder().uri("/api/providers").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn test_protected_routes_with_auth() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_keys_crud() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        // Create API key
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/api-keys")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "name": "test-key"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(create_response.status(), 200);

        // List API keys
        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/api-keys")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(list_response.status(), 200);
    }

    #[tokio::test]
    async fn test_chat_completion_requires_auth() {
        let (state, _) = setup_test_state().await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/chat/completions")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "model": "test",
                        "messages": [{"role": "user", "content": "hello"}]
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 401); // Auth middleware returns 401 when auth is missing
    }

    #[tokio::test]
    async fn test_api_auth_status() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/status")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_auth_logout() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/auth/logout")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_providers_crud() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        // Create provider
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .method("POST")
                    .header("authorization", format!("Bearer {}", token))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({
                        "name": "test-provider",
                        "slug": "test",
                        "base_url": "http://localhost:8080",
                        "api_key": "test-key",
                        "provider_type": "openai"
                    }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(create_response.status(), 200);

        // List providers
        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/providers")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(list_response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_config() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/config")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn test_api_metrics() {
        let (state, _) = setup_test_state().await;
        let token = setup_admin_user(&state).await;
        let app = create_test_app(state.clone()).await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .header("authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }
}
