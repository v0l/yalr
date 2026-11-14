use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::api::my_pricing::{self, CreateMyPrice, PriceInput};
use crate::api::my_providers::{self, CreateMyProvider, UpdateMyProvider};
use crate::auth::admin::{AuthenticatedUser, UserExtractor};
use crate::db::Database;
use crate::metrics::MetricsStore;
use crate::state::AppState;

async fn app() -> Arc<AppState> {
    let db = Arc::new(Database::test().await);
    let metrics_store = Arc::new(MetricsStore::new(100));
    let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
    let config = crate::config::AppConfig {
        db: db.clone(),
        router,
        auth_config: crate::auth::nip98::AuthConfig::default(),
        payments_config: None,
        discovery_config: None,
        admin_ui_path: String::new(),
        host: "0.0.0.0".to_string(),
        port: 3000,
    };
    Arc::new(AppState {
        config,
        metrics_emitter: metrics_store.emitter().clone(),
        metrics_store,
        session_store: Arc::new(crate::auth::admin::SessionStore::new()),
        db,
        payments_state: None,
        oauth_pending: Default::default(),
        model_cache: Default::default(),
        modality_cache: Default::default(),
        limits_cache: Default::default(),
    })
}

fn who(user: crate::db::User) -> UserExtractor {
    UserExtractor(AuthenticatedUser { user, api_key: None })
}

fn new_provider(slug: &str, url: &str) -> Json<CreateMyProvider> {
    Json(CreateMyProvider {
        name: slug.into(),
        slug: slug.into(),
        base_url: url.into(),
        api_key: Some("sk-very-secret".into()),
        provider_type: None,
    })
}

fn price(currency: &str) -> PriceInput {
    PriceInput {
        currency: currency.into(),
        price_per_1m_input: Some(100),
        price_per_1m_output: Some(200),
        price_per_1m_cache_read: None,
        price_per_1m_cache_write: None,
        price_per_request: None,
    }
}

#[tokio::test]
async fn private_urls_are_refused_at_save_time() {
    let state = app().await;
    let alice = state.db.add_test_user("alice", false).await;
    for url in ["http://127.0.0.1:9/v1", "http://10.0.0.1/v1", "http://169.254.169.254/", "http://100.64.1.1/"] {
        let err = my_providers::create(State(state.clone()), who(alice.clone()), new_provider("p", url))
            .await
            .err()
            .unwrap_or_else(|| panic!("{url} was accepted"));
        assert_eq!(err.0, StatusCode::BAD_REQUEST, "{url}");
    }
    assert!(state.db.list_providers_owned_by(alice.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_user_cannot_touch_another_users_provider() {
    let state = app().await;
    let alice = state.db.add_test_user("alice", false).await;
    let bob = state.db.add_test_user("bob", false).await;
    let created = my_providers::create(State(state.clone()), who(alice.clone()), new_provider("mine", "http://8.8.8.8/v1"))
        .await
        .unwrap()
        .0;

    let update = my_providers::update(
        State(state.clone()),
        who(bob.clone()),
        Path(created.slug.clone()),
        Json(UpdateMyProvider { name: Some("hijack".into()), base_url: None, api_key: None, provider_type: None }),
    )
    .await;
    assert_eq!(update.err().unwrap().0, StatusCode::NOT_FOUND);
    let delete = my_providers::delete(State(state.clone()), who(bob.clone()), Path(created.slug.clone())).await;
    assert_eq!(delete.err().unwrap().0, StatusCode::NOT_FOUND);
    assert_eq!(state.db.list_providers_owned_by(alice.id).await.unwrap().len(), 1);
    assert!(my_providers::list(State(state.clone()), who(bob)).await.unwrap().0.is_empty());
}

#[tokio::test]
async fn a_user_cannot_price_or_edit_prices_on_another_users_provider() {
    let state = app().await;
    let alice = state.db.add_test_user("alice", false).await;
    let bob = state.db.add_test_user("bob", false).await;
    let created = my_providers::create(State(state.clone()), who(alice.clone()), new_provider("mine", "http://8.8.8.8/v1"))
        .await
        .unwrap()
        .0;
    let request = |slug: &str| {
        Json(CreateMyPrice {
            provider_slug: slug.into(),
            model_name: "acme/m".into(),
            price: price("usd"),
            is_advertised: true,
        })
    };
    let foreign = my_pricing::create(State(state.clone()), who(bob.clone()), request(&created.slug)).await;
    assert_eq!(foreign.err().unwrap().0, StatusCode::NOT_FOUND);

    let mine = my_pricing::create(State(state.clone()), who(alice.clone()), request(&created.slug)).await.unwrap().0;
    assert_eq!(mine.owner_user_id, Some(alice.id));
    assert_eq!(mine.provider_slug.as_deref(), Some(created.slug.as_str()));
    assert_eq!(mine.currency, "usd");

    let edit = my_pricing::update(State(state.clone()), who(bob.clone()), Path(mine.id), Json(price("usd"))).await;
    assert_eq!(edit.err().unwrap().0, StatusCode::NOT_FOUND);
    let remove = my_pricing::delete(State(state.clone()), who(bob), Path(mine.id)).await;
    assert_eq!(remove.err().unwrap().0, StatusCode::NOT_FOUND);
    assert_eq!(state.db.model_pricing_by_id(mine.id).await.unwrap().price_per_1m_input_milli, Some(100));
}

#[tokio::test]
async fn provider_api_keys_never_appear_in_any_response() {
    let state = app().await;
    let alice = state.db.add_test_user("alice", false).await;
    let created = my_providers::create(State(state.clone()), who(alice.clone()), new_provider("mine", "http://8.8.8.8/v1"))
        .await
        .unwrap()
        .0;
    assert!(created.has_api_key);
    let listed = my_providers::list(State(state.clone()), who(alice.clone())).await.unwrap().0;
    let updated = my_providers::update(
        State(state.clone()),
        who(alice),
        Path(created.slug.clone()),
        Json(UpdateMyProvider { name: None, base_url: None, api_key: Some("sk-new-secret".into()), provider_type: None }),
    )
    .await
    .unwrap()
    .0;
    for body in [
        serde_json::to_string(&created).unwrap(),
        serde_json::to_string(&listed).unwrap(),
        serde_json::to_string(&updated).unwrap(),
    ] {
        assert!(!body.contains("secret"), "{body}");
        assert!(!body.contains("api_key\":\""), "{body}");
    }
}

#[tokio::test]
async fn admin_provider_listing_never_returns_keys() {
    let state = app().await;
    state
        .db
        .create_provider(crate::db::NewProvider {
            name: "plat",
            slug: "plat",
            base_url: "http://localhost:1",
            api_key: Some("sk-platform-secret"),
            provider_type: None,
        })
        .await
        .unwrap();
    let body = crate::api::providers::list_providers(State(state)).await.0;
    assert!(!serde_json::to_string(&body).unwrap().contains("secret"));
}
