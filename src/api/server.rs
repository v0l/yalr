use crate::api::chat::chat_handler;
use crate::api::{health, models as model_handlers, responses as responses_handlers};
use crate::api::ws;
use crate::auth::admin::SessionStore;
use crate::config::AppConfig;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::payments::PaymentsState;
use crate::state::AppState;
use axum::{
    body::Body,
    extract::DefaultBodyLimit,
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use super::compression::with_compression;

/// Body limit for inference requests. The axum default of 2 MB is far too small
/// for a long agent conversation: replayed reasoning content (which DeepSeek V4
/// and others require on every assistant turn) and base64 images push a single
/// request well past it, and the client sees
/// `413 Failed to buffer the request body: length limit exceeded` before any
/// routing happens. A long session with base64 image renders replayed on every
/// turn reached ~32 MB at 650k tokens (gcad fox benchmark, 2026-11-03), so the
/// limit is 64 MB.
pub(crate) const INFERENCE_BODY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// Upload cap for `/v1/audio/transcriptions`. OpenAI rejects anything over
/// 25 MB; local whisper backends accept more, so leave headroom for an hour of
/// uncompressed WAV.
const AUDIO_BODY_LIMIT_BYTES: usize = 256 * 1024 * 1024;

const IMAGE_BODY_LIMIT_BYTES: usize = 128 * 1024 * 1024;

async fn serve_admin_fallback(req: Request<Body>, admin_ui_path: String) -> impl IntoResponse {
    // Only serve admin UI for GET and HEAD requests
    if req.method() != axum::http::Method::GET && req.method() != axum::http::Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    
    // Skip API routes - they should return 404 if not found
    if path.starts_with("api/") || path.starts_with("v1/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    
    // Don't fall back to index.html for asset files (js, css, images, etc.)
    let is_asset = path.ends_with(".js") 
        || path.ends_with(".css") 
        || path.contains("/assets/")
        || path.contains("/favicon")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
        || path.ends_with(".svg")
        || path.ends_with(".ico");
    
    match tokio::fs::read(format!("{}/{}", admin_ui_path, path)).await {
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
            // For SPA routing, serve index.html for unknown paths (except assets)
            if is_asset {
                // Assets that don't exist should return 404
                StatusCode::NOT_FOUND.into_response()
            } else {
                // Unknown routes should serve index.html for SPA routing
                match tokio::fs::read(format!("{}/index.html", admin_ui_path)).await {
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
}

pub async fn run_with_shutdown<F>(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: std::sync::Arc<MetricsStore>,
    _shutdown: F,
) -> Result<(), Box<dyn std::error::Error>> {
    let session_store = Arc::new(SessionStore::new());
    let db = config.db.clone();

    // Initialize payments state if configured
    let payments_state = match config.payments_config.as_ref().filter(|pc| pc.enabled) {
        Some(pc) => match PaymentsState::new(pc.clone(), db.clone()).await {
            Ok(ps) => {
                tracing::info!("Payments system initialized");
                Some(Arc::new(ps))
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize payments, starting without payments");
                None
            }
        },
        None => None,
    };

    let state = Arc::new(AppState {
        config: config.clone(),
        metrics_emitter,
        metrics_store,
        session_store: session_store.clone(),
        db,
        payments_state,
        oauth_pending: Default::default(),
        model_cache: Default::default(),
        modality_cache: Default::default(),
        limits_cache: Default::default(),
        public_cache: Default::default(),
    });

    if crate::payouts::settings::get().enabled {
        crate::payouts::service::PayoutService::from_state(&state).spawn_worker();
    }

    if let Some(discovery) = config.discovery_config.clone() {
        crate::discovery::spawn(discovery, state.db.clone());
    }

    let warm = state.clone();
    tokio::spawn(async move {
        warm.public_cache.snapshot(&warm).await;
    });

    let app = build_app(state);

    let addr: SocketAddr = addr.parse()?;
    tracing::info!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown_signal()).await?;

    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await.ok();
    tracing::info!("Shutdown signal received, draining connections");
}

pub async fn run(
    config: AppConfig, 
    addr: &str,
    metrics_emitter: MetricsEmitter,
    metrics_store: std::sync::Arc<MetricsStore>,
) -> Result<(), Box<dyn std::error::Error>> {
    use tokio::signal;
    let shutdown = signal::ctrl_c();
    tokio::pin!(shutdown);
    run_with_shutdown(config, addr, metrics_emitter, metrics_store, shutdown).await
}

pub fn build_app(state: Arc<AppState>) -> Router {
    let auth = || axum::middleware::from_fn_with_state(state.clone(), crate::auth::admin::auth_middleware);
    let limited = |limit: usize| {
        (
            DefaultBodyLimit::max(limit),
            axum::middleware::from_fn_with_state(limit, crate::api::body_limit::request_too_large_json),
        )
    };

    let chat_completions_routes = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .layer(limited(INFERENCE_BODY_LIMIT_BYTES))
        .layer(auth());

    let responses_routes = Router::new()
        .route("/v1/responses", post(responses_handlers::create_response))
        .layer(limited(INFERENCE_BODY_LIMIT_BYTES))
        .layer(auth());

    let audio_routes = Router::new()
        .route("/v1/audio/transcriptions", post(crate::api::audio::transcriptions))
        .route("/v1/audio/translations", post(crate::api::audio::translations))
        .route("/v1/audio/speech", post(crate::api::audio::speech))
        .route("/v1/audio/voices", get(crate::api::audio::voices))
        .layer(limited(AUDIO_BODY_LIMIT_BYTES))
        .layer(auth());

    let image_routes = Router::new()
        .route("/v1/images/generations", post(crate::api::images::generations))
        .route("/v1/images/edits", post(crate::api::images::edits))
        .route("/v1/images/variations", post(crate::api::images::variations))
        .layer(limited(IMAGE_BODY_LIMIT_BYTES))
        .layer(auth());

    let decision_routes = Router::new()
        .route("/v1/systemone", post(crate::api::decisions::decide))
        .route("/api/alpha/decisions", post(crate::api::decisions::decide))
        .layer(limited(INFERENCE_BODY_LIMIT_BYTES))
        .layer(auth());

    let routstr_protected_routes = Router::new()
        .route("/v1/balance/info", get(crate::payments::routstr::balance_info))
        .route("/v1/account", get(crate::payments::account::account))
        .route("/v1/usage", get(crate::api::usage::my_usage))
        .route("/v1/quota", get(crate::api::quota::get_quota))
        .route("/v1/usage/requests", get(crate::api::usage::my_requests))
        .route("/v1/balance/refund", post(crate::payments::routstr::balance_refund))
        .route("/lightning/invoice", post(crate::payments::routstr::create_lightning_invoice))
        .route("/lightning/invoice/:payment_hash/status", get(crate::payments::routstr::check_lightning_invoice))
        .route("/providers/:slug/lightning/invoice", post(crate::payments::routstr::create_provider_invoice))
        .layer(auth());

    let models_route = Router::new()
        .route("/", get(model_handlers::models_entry))
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::auth::admin::optional_auth_middleware));

    let anonymous_payment_routes = Router::new()
        .route("/v2/lightning/invoice", post(crate::payments::anonymous::create_invoice))
        .route("/v2/lightning/invoice/:invoice_id/status", get(crate::payments::anonymous::invoice_status))
        .route("/v2/lightning/recover", post(crate::payments::anonymous::recover_invoice))
        .route("/v1/balance/lightning/invoice", post(crate::payments::anonymous::create_invoice))
        .route("/v1/balance/lightning/invoice/:invoice_id/status", get(crate::payments::anonymous::invoice_status));

    let admin_ui_path = state.config.admin_ui_path.clone();
    let app = Router::new()
        .nest("/api", crate::api::routes::api(state.clone()))
        .route("/api/metrics/ws", get(ws::ws_metrics_handler))
        .merge(chat_completions_routes)
        .merge(responses_routes)
        .merge(audio_routes)
        .merge(image_routes)
        .merge(decision_routes)
        .merge(routstr_protected_routes)
        .merge(anonymous_payment_routes)
        .nest("/v1/models", models_route)
        .route("/v1/info", get(crate::payments::routstr::routstr_info))
        .route("/api/health", get(health::health_check))
        .fallback(axum::routing::get(move |req: axum::extract::Request| serve_admin_fallback(req, admin_ui_path.clone())))
        .layer(CorsLayer::permissive())
        .layer(axum::middleware::map_response(crate::api::retry_after::add_header))
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    with_compression(app)
}

#[cfg(test)]
pub async fn create_test_app(state: Arc<AppState>) -> Router {
    build_app(state)
}
