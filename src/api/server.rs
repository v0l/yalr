use crate::api::chat::chat_handler;
use crate::api::{health, config, routing, providers, model_pricing, models as model_handlers, responses as responses_handlers, users};
use crate::api::ws;
use crate::auth::admin::SessionStore;
use crate::config::AppConfig;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::payments::PaymentsState;
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
    let payments_state = if let Some(ref pc) = config.payments_config {
        if pc.enabled {
            match PaymentsState::new(pc.clone(), db.clone()).await {
                Ok(ps) => {
                    tracing::info!("Payments system initialized");
                    Some(Arc::new(ps))
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to initialize payments, starting without payments");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let state = Arc::new(AppState {
        config: config.clone(),
        metrics_emitter,
        metrics_store,
        session_store: session_store.clone(),
        db,
        payments_state,
    });

    let admin_ui_path = config.admin_ui_path.clone();
    let cors = CorsLayer::permissive();

    use crate::auth::admin::{login, logout, auth_status, check_setup_complete, setup_first_user, auth_middleware, admin_middleware};
    use crate::auth::api_keys::{create_api_key, list_api_keys, delete_api_key, disable_api_key, enable_api_key, create_api_key_for_user};
    use users::{list_users, create_user, update_user, delete_user, get_user, list_all_balances, get_user_balance_details, admin_credit_user, admin_debit_user, list_admin_transactions, list_admin_invoices, list_user_model_permissions, create_user_model_permission, delete_user_model_permission};
    
    let public_auth_routes = Router::new()
        .route("/auth/setup", post(setup_first_user))
        .route("/auth/login", post(login))
        .route("/setup/status", get(check_setup_complete));

    let protected_routes = Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/logout", post(logout))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let admin_routes = Router::new()
        .route("/providers", get(providers::list_providers))
        .route("/providers", post(providers::create_provider))
        .route("/providers/:slug/topup", post(providers::create_provider_topup))
        .route("/providers/:slug", put(providers::update_provider))
        .route("/providers/:slug/generate-api-key", post(providers::generate_provider_api_key))
        .route("/providers/:slug", delete(providers::delete_provider))
        .route("/metrics", get(health::get_metrics))
        .route("/metrics/history", get(health::get_metrics_history))
        .route("/metrics/health", get(health::get_health_overview))
        .route("/config", get(config::get_router_config))
        .route("/routing-configs", get(routing::list_routing_configs))
        .route("/routing-configs", post(routing::create_routing_config))
        .route("/routing-configs/:id", put(routing::update_routing_config))
        .route("/routing-configs/:id", delete(routing::delete_routing_config))
        .route("/routing-configs/providers", post(routing::create_routing_config_provider))
        .route("/routing-configs/providers/:id", put(routing::update_routing_config_provider))
        .route("/routing-configs/providers/:id", delete(routing::delete_routing_config_provider))
        .route("/providers/:slug/models", get(model_handlers::list_provider_models))
        .route("/models/sync/:provider_slug", get(model_handlers::sync_provider_models))
        .route("/models/discrepancies", post(model_handlers::detect_model_discrepancies))
        .route("/model-pricing", get(model_pricing::list_model_pricing))
        .route("/model-pricing", post(model_pricing::create_model_pricing))
        .route("/model-pricing/:model_name", put(model_pricing::update_model_pricing))
        .route("/model-pricing/:model_name", delete(model_pricing::delete_model_pricing))
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
        .route("/payments/balances", get(list_all_balances))
        .route("/payments/balances/:user_id", get(get_user_balance_details))
        .route("/payments/credit", post(admin_credit_user))
        .route("/payments/debit", post(admin_debit_user))
        .route("/payments/transactions", get(list_admin_transactions))
        .route("/payments/invoices", get(list_admin_invoices))
        .route("/users/:user_id/models", get(list_user_model_permissions))
        .route("/users/:user_id/models", post(create_user_model_permission))
        .route("/users/:user_id/models/:model", delete(delete_user_model_permission))
        .layer(axum::middleware::from_fn_with_state(state.clone(), admin_middleware));

    let all_protected = protected_routes.merge(admin_routes);

    let chat_completions_routes = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let responses_routes = Router::new()
        .route("/v1/responses", post(responses_handlers::create_response))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let routstr_protected_routes = Router::new()
        .route("/v1/balance/info", get(crate::payments::routstr::balance_info))
        .route("/v1/balance/refund", post(crate::payments::routstr::balance_refund))
        .route("/lightning/invoice", post(crate::payments::routstr::create_lightning_invoice))
        .route("/lightning/invoice/:payment_hash/status", get(crate::payments::routstr::check_lightning_invoice))
        .route("/providers/:slug/lightning/invoice", post(crate::payments::routstr::create_provider_invoice))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let models_route = Router::new()
        .route("/", get(model_handlers::list_models))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let app = Router::new()
        .nest("/api", public_auth_routes.merge(all_protected))
        .route("/api/metrics/ws", get(ws::ws_metrics_handler))
        .merge(chat_completions_routes)
        .merge(responses_routes)
        .merge(routstr_protected_routes)
        .nest("/v1/models", models_route)
        .route("/v1/info", get(crate::payments::routstr::routstr_info))
        .route("/api/health", get(health::health_check))
        .fallback(axum::routing::get({
            let admin_ui_path = admin_ui_path.clone();
            move |req: axum::extract::Request| serve_admin_fallback(req, admin_ui_path.clone())
        }))
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
    metrics_store: std::sync::Arc<MetricsStore>,
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
    use users::{list_users, create_user, update_user, delete_user, get_user, list_all_balances, get_user_balance_details, admin_credit_user, admin_debit_user, list_admin_transactions, list_admin_invoices};
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
        .route("/providers", get(providers::list_providers))
        .route("/providers", post(providers::create_provider))
        .route("/providers/:slug", put(providers::update_provider))
        .route("/providers/:slug", delete(providers::delete_provider))
        .route("/metrics", get(health::get_metrics))
        .route("/metrics/history", get(health::get_metrics_history))
        .route("/metrics/health", get(health::get_health_overview))
        .route("/config", get(config::get_router_config))
        .route("/routing-configs", get(routing::list_routing_configs))
        .route("/routing-configs", post(routing::create_routing_config))
        .route("/routing-configs/:id", put(routing::update_routing_config))
        .route("/routing-configs/:id", delete(routing::delete_routing_config))
        .route("/routing-configs/providers", post(routing::create_routing_config_provider))
        .route("/routing-configs/providers/:id", put(routing::update_routing_config_provider))
        .route("/routing-configs/providers/:id", delete(routing::delete_routing_config_provider))
        .route("/providers/:slug/models", get(model_handlers::list_provider_models))
        .route("/models/sync/:provider_slug", get(model_handlers::sync_provider_models))
        .route("/models/discrepancies", post(model_handlers::detect_model_discrepancies))
        .route("/model-pricing", get(model_pricing::list_model_pricing))
        .route("/model-pricing", post(model_pricing::create_model_pricing))
        .route("/model-pricing/:model_name", put(model_pricing::update_model_pricing))
        .route("/model-pricing/:model_name", delete(model_pricing::delete_model_pricing))
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
        .route("/payments/balances", get(list_all_balances))
        .route("/payments/balances/:user_id", get(get_user_balance_details))
        .route("/payments/credit", post(admin_credit_user))
        .route("/payments/debit", post(admin_debit_user))
        .route("/payments/transactions", get(list_admin_transactions))
        .route("/payments/invoices", get(list_admin_invoices))
        .layer(axum::middleware::from_fn_with_state(state.clone(), admin_middleware));

    let all_protected = protected_routes.merge(admin_routes);

    let chat_completions_routes = Router::new()
        .route("/v1/chat/completions", post(chat_handler))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let responses_routes = Router::new()
        .route("/v1/responses", post(responses_handlers::create_response))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let routstr_protected_routes = Router::new()
        .route("/v1/balance/info", get(crate::payments::routstr::balance_info))
        .route("/v1/balance/refund", post(crate::payments::routstr::balance_refund))
        .route("/lightning/invoice", post(crate::payments::routstr::create_lightning_invoice))
        .route("/lightning/invoice/:payment_hash/status", get(crate::payments::routstr::check_lightning_invoice))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    let models_route = Router::new()
        .route("/", get(model_handlers::list_models))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));

    Router::new()
        .merge(chat_completions_routes)
        .merge(responses_routes)
        .merge(routstr_protected_routes)
        .nest("/v1/models", models_route)
        .route("/v1/info", get(crate::payments::routstr::routstr_info))
        .route("/api/health", get(health::health_check))
        .route("/api/metrics/ws", get(ws::ws_metrics_handler))
        .nest("/api", public_auth_routes.merge(all_protected))
        .fallback_service(axum::routing::get({
            let state = state.clone();
            move || serve_admin_fallback(axum::extract::Request::new(Body::empty()), state.config.admin_ui_path.clone())
        }))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}