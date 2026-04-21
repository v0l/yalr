use crate::api::handlers::{self, chat_handler};
use crate::auth::admin::SessionStore;
use crate::config::AppConfig;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::state::AppState;
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{trace::TraceLayer, cors::CorsLayer, services::ServeDir};

pub async fn run(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: MetricsStore,
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

    use crate::auth::admin::{login, logout, auth_status, check_setup_complete, setup_first_user, auth_middleware};
    use crate::auth::api_keys::{create_api_key, list_api_keys, delete_api_key, disable_api_key, enable_api_key};
    
    let public_auth_routes = Router::new()
        .route("/auth/setup", post(setup_first_user))
        .route("/auth/login", post(login))
        .route("/setup/status", get(check_setup_complete));

    let protected_routes = Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/logout", post(logout))
        .route("/providers", get(handlers::list_providers))
        .route("/providers", post(handlers::create_provider))
        .route("/providers/:slug", delete(handlers::delete_provider))
        .route("/metrics", get(handlers::get_metrics))
        .route("/config", get(handlers::get_router_config))
        .route("/models/sync/:provider_slug", get(handlers::sync_provider_models))
        .route("/models/discrepancies", post(handlers::detect_model_discrepancies))
        .route("/api-keys", get(list_api_keys))
        .route("/api-keys", post(create_api_key))
        .route("/api-keys/:id", delete(delete_api_key))
        .route("/api-keys/:id/disable", post(disable_api_key))
        .route("/api-keys/:id/enable", post(enable_api_key))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Serve static files from admin dist directory with SPA fallback
    let admin_dist_path = "/app/admin/dist";
    let serve_admin = ServeDir::new(admin_dist_path)
        .precompressed_gzip()
        .precompressed_br();

    let app = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .route("/v1/models", get(handlers::list_models))
        .route("/health", get(handlers::health_check))
        .nest("/api", public_auth_routes.merge(protected_routes))
        .fallback_service(serve_admin)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = addr.parse()?;
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
