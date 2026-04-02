use crate::api::handlers::{self, chat_handler};
use crate::config::AppConfig;
use crate::metrics::{MetricsEmitter, MetricsStore};
use axum::{
    routing::{delete, get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub metrics_emitter: MetricsEmitter,
    pub metrics_store: MetricsStore,
}

pub fn create_router(config: AppConfig, metrics_store: MetricsStore) -> Router<AppState> {
    let state = AppState {
        config,
        metrics_emitter: metrics_store.emitter().clone(),
        metrics_store,
    };

    Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .route("/v1/models", get(handlers::list_models))
        .route("/admin/providers", get(handlers::list_providers))
        .route("/admin/providers", post(handlers::create_provider))
        .route("/admin/providers/:slug", delete(handlers::delete_provider))
        .route("/admin/metrics", get(handlers::get_metrics))
        .route("/health", get(handlers::health_check))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn run(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: MetricsStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        config,
        metrics_emitter,
        metrics_store,
    };

    let app = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .route("/v1/models", get(handlers::list_models))
        .route("/providers", get(handlers::list_providers))
        .route("/providers", post(handlers::create_provider))
        .route("/providers/:slug", delete(handlers::delete_provider))
        .route("/health", get(handlers::health_check))
        .route("/metrics", get(handlers::get_metrics))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr: SocketAddr = addr.parse()?;
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
