use crate::api::handlers::{self, chat_handler};
use crate::api::ws;
use crate::auth::admin::SessionStore;
use crate::config::AppConfig;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::state::AppState;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{trace::TraceLayer, cors::CorsLayer};

async fn serve_admin_fallback(req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    
    match tokio::fs::read(format!("/app/admin/dist/{}", path)).await {
        Ok(contents) => {
            let content_type = if path.ends_with(".html") {
                "text/html; charset=utf-8"
            } else if path.ends_with(".css") {
                "text/css"
            } else if path.ends_with(".js") {
                "application/javascript"
            } else {
                "application/octet-stream"
            };
            let mut headers = axum::http::HeaderMap::new();
            headers.insert(axum::http::header::CONTENT_TYPE, content_type.parse().unwrap());
            (headers, contents).into_response()
        },
        Err(_) => {
            // For SPA routing, serve index.html for any unknown path
            match tokio::fs::read("/app/admin/dist/index.html").await {
                Ok(contents) => {
                    let mut headers = axum::http::HeaderMap::new();
                    headers.insert(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
                    (headers, contents).into_response()
                },
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
    }
}

pub async fn run_with_shutdown<F>(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: MetricsStore,
    _shutdown: F,
) -> Result<(), Box<dyn std::error::Error>> {
    let session_store = Arc::new(SessionStore::new());
    let db = config.db.clone();
    let state = Arc::new(AppState {
        config: config.clone(),
        metrics_emitter,
        metrics_store,
        session_store: session_store.clone(),
        db,
    });

    let cors = CorsLayer::permissive();

    use crate::auth::admin::{login, logout, auth_status, check_setup_complete, setup_first_user, auth_middleware, admin_middleware};
    use crate::auth::api_keys::{create_api_key, list_api_keys, delete_api_key, disable_api_key, enable_api_key, create_api_key_for_user};
    use handlers::{list_users, create_user, update_user, delete_user, get_user};
    
    let public_auth_routes = Router::new()
        .route("/auth/setup", post(setup_first_user))
        .route("/auth/login", post(login))
        .route("/setup/status", get(check_setup_complete));

    let protected_routes = Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/logout", post(logout))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let admin_routes = Router::new()
        .route("/providers", get(handlers::list_providers))
        .route("/providers", post(handlers::create_provider))
        .route("/providers/:slug", put(handlers::update_provider))
        .route("/providers/:slug", delete(handlers::delete_provider))
        .route("/metrics", get(handlers::get_metrics))
        .route("/config", get(handlers::get_router_config))
        .route("/routing-configs", get(handlers::list_routing_configs))
        .route("/routing-configs", post(handlers::create_routing_config))
        .route("/routing-configs/:id", put(handlers::update_routing_config))
        .route("/routing-configs/:id", delete(handlers::delete_routing_config))
        .route("/routing-configs/providers", post(handlers::create_routing_config_provider))
        .route("/routing-configs/providers/:id", put(handlers::update_routing_config_provider))
        .route("/routing-configs/providers/:id", delete(handlers::delete_routing_config_provider))
        .route("/models/sync/:provider_slug", get(handlers::sync_provider_models))
        .route("/models/discrepancies", post(handlers::detect_model_discrepancies))
        .route("/api-keys", get(list_api_keys))
        .route("/api-keys", post(create_api_key))
        .route("/api-keys/:id", delete(delete_api_key))
        .route("/api-keys/:id/disable", post(disable_api_key))
        .route("/api-keys/:id/enable", post(enable_api_key))
        .route("/users", get(list_users))
        .route("/users", post(create_user))
        .route("/users/:id/api-keys", post(create_api_key_for_user))
        .route("/users/:id", get(get_user))
        .route("/users/:id", put(update_user))
        .route("/users/:id", delete(delete_user))
        .layer(axum::middleware::from_fn_with_state(state.clone(), admin_middleware));

    let all_protected = protected_routes.merge(admin_routes);

    let chat_completions_routes = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let responses_routes = Router::new()
        .route("/v1/responses", post(handlers::create_response))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let app = Router::new()
        .nest("/api", public_auth_routes.merge(all_protected))
        .route("/api/metrics/ws", get(ws::ws_metrics_handler))
        .merge(chat_completions_routes)
        .merge(responses_routes)
        .route("/v1/models", get(handlers::list_models))
        .route("/api/health", get(handlers::health_check))
        .fallback(serve_admin_fallback)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = addr.parse()?;
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
    }).await?;

    Ok(())
}

pub async fn run(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: MetricsStore,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::signal;
    let shutdown = signal::ctrl_c();
    tokio::pin!(shutdown);
    run_with_shutdown(config, addr, metrics_emitter, metrics_store, shutdown).await
}

#[cfg(test)]
pub async fn create_test_app(state: Arc<AppState>) -> Router {
    use crate::auth::admin::{login, logout, auth_status, check_setup_complete, setup_first_user, auth_middleware, admin_middleware, AdminExtractor};
    use crate::auth::api_keys::{create_api_key, list_api_keys, delete_api_key, disable_api_key, enable_api_key, create_api_key_for_user};
    use handlers::{list_users, create_user, update_user, delete_user, get_user};
    use tower_http::cors::CorsLayer;
    use tower_http::trace::TraceLayer;
    
    let public_auth_routes = Router::new()
        .route("/auth/setup", post(setup_first_user))
        .route("/auth/login", post(login))
        .route("/setup/status", get(check_setup_complete));

    let protected_routes = Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/logout", post(logout))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let admin_routes = Router::new()
        .route("/providers", get(handlers::list_providers))
        .route("/providers", post(handlers::create_provider))
        .route("/providers/:slug", put(handlers::update_provider))
        .route("/providers/:slug", delete(handlers::delete_provider))
        .route("/metrics", get(handlers::get_metrics))
        .route("/config", get(handlers::get_router_config))
        .route("/routing-configs", get(handlers::list_routing_configs))
        .route("/routing-configs", post(handlers::create_routing_config))
        .route("/routing-configs/:id", put(handlers::update_routing_config))
        .route("/routing-configs/:id", delete(handlers::delete_routing_config))
        .route("/routing-configs/providers", post(handlers::create_routing_config_provider))
        .route("/routing-configs/providers/:id", put(handlers::update_routing_config_provider))
        .route("/routing-configs/providers/:id", delete(handlers::delete_routing_config_provider))
        .route("/models/sync/:provider_slug", get(handlers::sync_provider_models))
        .route("/models/discrepancies", post(handlers::detect_model_discrepancies))
        .route("/api-keys", get(list_api_keys))
        .route("/api-keys", post(create_api_key))
        .route("/api-keys/:id", delete(delete_api_key))
        .route("/api-keys/:id/disable", post(disable_api_key))
        .route("/api-keys/:id/enable", post(enable_api_key))
        .route("/users", get(list_users))
        .route("/users", post(create_user))
        .route("/users/:id/api-keys", post(create_api_key_for_user))
        .route("/users/:id", get(get_user))
        .route("/users/:id", put(update_user))
        .route("/users/:id", delete(delete_user))
        .layer(axum::middleware::from_fn_with_state(state.clone(), admin_middleware));

    let all_protected = protected_routes.merge(admin_routes);

    let chat_completions_routes = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let responses_routes = Router::new()
        .route("/v1/responses", post(handlers::create_response))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    Router::new()
        .merge(chat_completions_routes)
        .merge(responses_routes)
        .route("/v1/models", get(handlers::list_models))
        .route("/api/health", get(handlers::health_check))
        .route("/api/metrics/ws", get(ws::ws_metrics_handler))
        .nest("/api", public_auth_routes.merge(all_protected))
        .fallback(serve_admin_fallback)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
