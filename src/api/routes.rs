use crate::api::{config, health, model_pricing, models as model_handlers, provider_payments, provider_stats, providers, routing, users};
use crate::auth::admin::{
    admin_middleware, auth_middleware, auth_status, check_setup_complete, login, logout, setup_first_user,
};
use crate::auth::api_keys::{create_api_key, create_api_key_for_user, disable_api_key, enable_api_key, list_api_keys};
use crate::state::AppState;
use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

pub fn api(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let router = public().merge(tenant(state.clone())).merge(admin(state.clone()));
    if state.payments_state.as_ref().is_some_and(|p| p.revolut.is_some()) {
        return router.merge(fiat(state));
    }
    router
}

fn fiat(state: Arc<AppState>) -> Router<Arc<AppState>> {
    use crate::api::fiat;
    let tenant = Router::new()
        .route("/account/topup/fiat/config", get(fiat::config))
        .route("/account/topup/fiat", get(fiat::history).post(fiat::create_topup))
        .route("/account/topup/fiat/:id", get(fiat::topup_status))
        .route("/account/payment-methods", get(fiat::list_cards))
        .route("/account/payment-methods/:id", delete(fiat::delete_card))
        .route("/account/payment-methods/:id/default", put(fiat::default_card))
        .route("/account/auto-reload", get(fiat::list_auto_reload).put(fiat::save_auto_reload))
        .layer(axum::middleware::from_fn_with_state(state.clone(), auth_middleware));
    let admin = Router::new()
        .route("/payments/fiat-orders", get(fiat::admin_orders))
        .layer(axum::middleware::from_fn_with_state(state, admin_middleware));
    Router::new().route("/webhooks/revolut", post(fiat::webhook)).merge(tenant).merge(admin)
}

fn public() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/setup", post(setup_first_user))
        .route("/auth/login", post(login))
        .route("/auth/register", post(crate::auth::signup::register))
        .route("/auth/nostr", post(crate::auth::signup::nostr_login))
        .route("/setup/status", get(check_setup_complete))
        .route("/public/models", get(crate::api::public_models::list))
        .route("/public/models/*name", get(crate::api::public_models::detail))
}

fn tenant(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/status", get(auth_status))
        .route("/auth/logout", post(logout))
        .route("/providers", get(providers::list_providers).post(providers::create_provider))
        .route("/providers/:slug", put(providers::update_provider).delete(providers::delete_provider))
        .route("/providers/:slug/models", get(model_handlers::list_provider_models))
        .route("/providers/:slug/listing", get(crate::api::listing_status::provider_listing))
        .route("/providers/:slug/stats", get(provider_stats::stats))
        .route("/providers/:slug/check", post(provider_stats::check))
        .route("/providers/:slug/verify", post(crate::api::provider_approval::verify))
        .route("/providers/:slug/verifications", get(crate::api::provider_approval::verifications))
        .route("/providers/:slug/topup", post(provider_payments::create_provider_topup))
        .route("/providers/:slug/generate-api-key", post(provider_payments::generate_provider_api_key))
        .route("/metrics", get(health::get_metrics))
        .route("/metrics/history", get(health::get_metrics_history))
        .route("/metrics/health", get(health::get_health_overview))
        .route("/request-log", get(crate::api::request_log::list))
        .route("/request-log/:id", get(crate::api::request_log::detail))
        .route("/request-log/:id/bodies", get(crate::api::request_log::bodies))
        .route("/routing-configs", get(routing::list_routing_configs).post(routing::create_routing_config))
        .route("/routing-configs/:id", put(routing::update_routing_config).delete(routing::delete_routing_config))
        .route("/routing-configs/providers", post(routing::create_routing_config_provider))
        .route(
            "/routing-configs/providers/:id",
            put(routing::update_routing_config_provider).delete(routing::delete_routing_config_provider),
        )
        .route("/model-pricing", get(model_pricing::list_model_pricing).post(model_pricing::create_model_pricing))
        .route(
            "/model-pricing/id/:id",
            put(model_pricing::update_pricing_by_id).delete(model_pricing::delete_pricing_by_id),
        )
        .route("/model-pricing/:model_name", put(model_pricing::update_model_pricing).delete(model_pricing::delete_model_pricing))
        .route("/payments/balances", get(users::list_all_balances))
        .route("/payments/transactions", get(users::list_admin_transactions))
        .route("/payments/invoices", get(users::list_admin_invoices))
        .route("/payments/currencies", get(crate::api::currency_balances::currencies))
        .route("/my/balances", get(crate::api::currency_balances::my_balances))
        .route("/api-keys", get(list_api_keys).post(create_api_key))
        .route("/api-keys/:id", delete(crate::auth::api_keys::delete_my_api_key))
        .layer(axum::middleware::from_fn_with_state(state, auth_middleware))
}

fn admin(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/oauth/start", post(crate::api::oauth::start))
        .route("/oauth/complete", post(crate::api::oauth::complete))
        .route("/oauth/:slug/reauth", post(crate::api::oauth::reauth))
        .route("/config", get(config::get_router_config))
        .route("/providers/:slug/status", put(crate::api::provider_approval::set_status))
        .route("/providers/:slug/reference", put(crate::api::provider_approval::set_reference))
        .route("/models/sync/:provider_slug", get(model_handlers::sync_provider_models))
        .route("/models/discrepancies", post(model_handlers::detect_model_discrepancies))
        .route("/model-pricing/market", get(crate::api::market::market))
        .route("/model-pricing/listing", get(crate::api::market::listing))
        .route("/api-keys/:id/disable", post(disable_api_key))
        .route("/api-keys/:id/enable", post(enable_api_key))
        .route("/api-keys/:id/usage", get(crate::api::usage::key_usage))
        .route("/users/:id/usage", get(crate::api::usage::user_usage))
        .route("/users", get(users::list_users).post(users::create_user))
        .route("/users/:id/api-keys", post(create_api_key_for_user))
        .route("/users/:id", get(users::get_user).put(users::update_user).delete(users::delete_user))
        .route("/payments/balances/:user_id", get(users::get_user_balance_details))
        .route("/payments/balances/:user_id/currencies", get(crate::api::currency_balances::user_balances))
        .route("/payments/credit", post(crate::api::currency_balances::admin_credit))
        .route("/payments/debit", post(crate::api::currency_balances::admin_debit))
        .route("/users/:user_id/models", get(users::list_user_model_permissions).post(users::create_user_model_permission))
        .route("/users/:user_id/models/:model", delete(users::delete_user_model_permission))
        .layer(axum::middleware::from_fn_with_state(state, admin_middleware))
}
