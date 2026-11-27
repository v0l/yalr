use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::util::ServiceExt;

use crate::db::{Database, NewProvider, Record};
use crate::metrics::MetricsStore;
use crate::state::AppState;

pub(super) struct Tenancy {
    pub(super) app: axum::Router,
    pub(super) state: Arc<AppState>,
    pub(super) alice: String,
    pub(super) bob: String,
    pub(super) admin: String,
}

pub(super) async fn tenancy() -> Tenancy {
    build_tenancy(None, None, None).await
}

pub(super) async fn tenancy_with_gateway(url: &str) -> Tenancy {
    let settings = crate::config::GatewaySettings { url: url.to_string(), sandbox: true, ..Default::default() };
    build_tenancy(Some(Default::default()), Some(settings), None).await
}

pub(super) async fn tenancy_with_lnd(node: Arc<crate::payments::lnd::fake::FakeLnd>) -> Tenancy {
    build_tenancy(Some(Default::default()), None, Some(node)).await
}

pub(super) async fn tenancy_with_payments(defaults: crate::config::DefaultPricingConfig) -> Tenancy {
    build_tenancy(Some(defaults), None, None).await
}

async fn build_tenancy(
    defaults: Option<crate::config::DefaultPricingConfig>,
    gateway: Option<crate::config::GatewaySettings>,
    lnd: Option<Arc<crate::payments::lnd::fake::FakeLnd>>,
) -> Tenancy {
    let db = Arc::new(Database::test().await);
    let payments_state = defaults.map(|d| {
        Arc::new(crate::payments::PaymentsState {
            config: Default::default(),
            balance_service: Arc::new(crate::payments::balance::BalanceService::new(db.clone())),
            pricing_resolver: Arc::new(crate::payments::pricing::PricingResolver::new(db.clone(), d)),
            gateway: gateway.map(|settings| {
                let secrets = crate::payments::gateway::GatewaySecrets {
                    api_key: "pgw_test".into(),
                    webhook_secret: crate::payments::gateway::fake::WEBHOOK_SECRET.into(),
                };
                Arc::new(crate::payments::gateway::GatewayService::new(settings, secrets, db.clone()).unwrap())
            }),
            lnd: lnd.map(|node| {
                let settings = crate::config::LndSettings::new("https://lnd.test", "cert", "macaroon");
                Arc::new(crate::payments::lnd::LndService::new(node, db.clone(), settings))
            }),
        })
    });
    let metrics_store = Arc::new(MetricsStore::new(100));
    let router = Arc::new(
        crate::router::engine::Router::new((*metrics_store).clone(), db.clone())
            .with_egress(crate::providers::egress::Egress::loopback_for_tests()),
    );
    let sessions = Arc::new(crate::auth::admin::SessionStore::new());
    let state = Arc::new(AppState {
        config: crate::config::AppConfig {
            db: db.clone(),
            router,
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
            discovery_config: None,
            admin_ui_path: String::new(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        },
        metrics_emitter: metrics_store.emitter().clone(),
        metrics_store,
        session_store: sessions.clone(),
        db,
        payments_state,
        oauth_pending: Default::default(),
        model_cache: Default::default(),
        modality_cache: Default::default(),
        limits_cache: Default::default(),
        public_cache: Default::default(),
    });
    for (name, admin) in [("alice", false), ("bob", false), ("root", true)] {
        state.db.add_test_user(name, admin).await;
    }
    Tenancy {
        app: crate::api::server::create_test_app(state.clone()).await,
        alice: sessions.create("alice", false, 3600).await,
        bob: sessions.create("bob", false, 3600).await,
        admin: sessions.create("root", true, 3600).await,
        state,
    }
}

impl Tenancy {
    pub(super) async fn call(&self, token: &str, method: &str, uri: &str, body: &str) -> (u16, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .method(method)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::String(String::from_utf8_lossy(&bytes).into())))
    }

    pub(super) async fn get(&self, token: &str, uri: &str) -> (u16, Value) {
        self.call(token, "GET", uri, "").await
    }

    pub(super) async fn alice_provider(&self) -> String {
        let (status, body) = self
            .call(
                &self.alice,
                "POST",
                "/api/providers",
                r#"{"name":"gpu","slug":"gpu","base_url":"http://8.8.8.8/v1","api_key":"sk-very-secret","provider_type":"openai"}"#,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        body["slug"].as_str().unwrap().to_string()
    }
}

fn slugs(body: &Value) -> Vec<String> {
    body["providers"].as_array().unwrap().iter().map(|p| p["slug"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn a_regular_user_reaches_the_dashboard_pages_and_not_the_admin_ones() {
    let t = tenancy().await;
    for uri in ["/api/providers", "/api/api-keys", "/api/metrics", "/api/metrics/health", "/api/metrics/history", "/api/request-log", "/api/routing-configs", "/api/model-pricing", "/api/payments/balances", "/api/payments/transactions", "/api/payments/invoices"] {
        assert_eq!(t.get(&t.alice, uri).await.0, 200, "{uri}");
    }
    for uri in ["/api/users", "/api/config", "/api/payments/balances/1"] {
        assert_eq!(t.get(&t.alice, uri).await.0, 403, "{uri}");
    }
}

#[tokio::test]
async fn providers_are_namespaced_and_invisible_to_other_tenants() {
    let t = tenancy().await;
    t.state
        .db
        .create_provider(NewProvider { name: "plat", slug: "plat", base_url: "http://localhost:1", api_key: Some("sk-platform-secret"), provider_type: None })
        .await
        .unwrap();
    let slug = t.alice_provider().await;
    assert!(slug.starts_with('u') && slug.ends_with("-gpu"), "{slug}");

    assert_eq!(slugs(&t.get(&t.alice, "/api/providers").await.1), vec![slug.clone()]);
    assert!(slugs(&t.get(&t.bob, "/api/providers").await.1).is_empty());
    let all = slugs(&t.get(&t.admin, "/api/providers").await.1);
    assert!(all.contains(&slug) && all.contains(&"plat".to_string()));

    for (method, uri) in [
        ("PUT", format!("/api/providers/{slug}")),
        ("DELETE", format!("/api/providers/{slug}")),
        ("GET", format!("/api/providers/{slug}/stats")),
        ("GET", format!("/api/providers/{slug}/models")),
        ("POST", format!("/api/providers/{slug}/check")),
        ("PUT", "/api/providers/plat".to_string()),
    ] {
        assert_eq!(t.call(&t.bob, method, &uri, r#"{"name":"hijack"}"#).await.0, 404, "{method} {uri}");
    }
    assert_eq!(t.get(&t.alice, &format!("/api/providers/{slug}/stats")).await.0, 200);
}

#[tokio::test]
async fn tenants_cannot_take_over_slugs_owners_or_private_urls() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let (status, _) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"slug":"plat"}"#).await;
    assert_eq!(status, 400);
    let (status, _) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"owner_user_id":null}"#).await;
    assert_eq!(status, 400);
    for url in ["http://127.0.0.1:9/v1", "http://10.0.0.1/v1", "http://169.254.169.254/"] {
        let body = format!(r#"{{"name":"p","slug":"p","base_url":"{url}","provider_type":"openai"}}"#);
        assert_eq!(t.call(&t.alice, "POST", "/api/providers", &body).await.0, 400, "{url}");
    }
    for (url, why) in [
        ("https://example.com#@127.0.0.1/v1", "fragment"),
        ("https://example.com:22/v1", "port 22"),
        ("http://0a2a0005.8ac540d4.rbndr.us:6379/v1", "port 6379"),
        ("https://user:pw@8.8.8.8/v1", "user name"),
        ("http://8.8.8.8@127.0.0.1/v1", "user name"),
        ("http://8.8.8.8:5432/v1", "port 5432"),
    ] {
        let body = format!(r#"{{"name":"p","slug":"p","base_url":"{url}","provider_type":"openai"}}"#);
        let (status, reply) = t.call(&t.alice, "POST", "/api/providers", &body).await;
        assert_eq!(status, 400, "{url}");
        assert!(reply.to_string().contains(why), "{url}: {reply}");
        let (status, _) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), &format!(r#"{{"base_url":"{url}"}}"#)).await;
        assert_eq!(status, 400, "update {url}");
    }
    let (_, private) = t.call(&t.alice, "POST", "/api/providers", r#"{"name":"p","slug":"p","base_url":"http://10.42.0.5/v1"}"#).await;
    assert!(!private.to_string().contains("10.42"), "{private}");
    let oauth = r#"{"name":"p","slug":"p","base_url":"https://api.anthropic.com","provider_type":"anthropic-oauth"}"#;
    assert_eq!(t.call(&t.alice, "POST", "/api/providers", oauth).await.0, 400);
}

#[tokio::test]
async fn provider_api_keys_never_appear_in_any_response() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let (_, updated) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"api_key":"sk-new-secret"}"#).await;
    let (_, listed) = t.get(&t.alice, "/api/providers").await;
    let (_, admin_listed) = t.get(&t.admin, "/api/providers").await;
    assert_eq!(listed["providers"][0]["has_api_key"], true);
    for body in [updated, listed, admin_listed] {
        assert!(!body.to_string().contains("secret"), "{body}");
    }
}

#[tokio::test]
async fn routing_configs_are_owned_and_take_only_the_owners_providers() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let platform = t.state.db.create_provider(NewProvider { name: "plat", slug: "plat", base_url: "http://localhost:1", api_key: None, provider_type: None }).await.unwrap();
    let alice_provider = t.state.db.get_provider_by_slug(&slug).await.unwrap().unwrap();
    let config = r#"{"name":"fast","strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5}"#;
    let (status, created) = t.call(&t.alice, "POST", "/api/routing-configs", config).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["name"], "alice/fast");
    let config_id = created["id"].as_i64().unwrap();

    let assign = |provider_id: i64| format!(r#"{{"routing_config_id":{config_id},"provider_id":{provider_id},"weight":1,"is_active":true}}"#);
    assert_eq!(t.call(&t.alice, "POST", "/api/routing-configs/providers", &assign(alice_provider.id)).await.0, 200);
    assert_eq!(t.call(&t.alice, "POST", "/api/routing-configs/providers", &assign(platform.id)).await.0, 404);
    assert_eq!(t.call(&t.bob, "POST", "/api/routing-configs/providers", &assign(alice_provider.id)).await.0, 404);

    let (_, bob_configs) = t.get(&t.bob, "/api/routing-configs").await;
    assert_eq!(bob_configs.as_array().unwrap().len(), 0);
    assert_eq!(t.call(&t.bob, "DELETE", &format!("/api/routing-configs/{config_id}"), "").await.0, 404);
    let (_, mine) = t.get(&t.alice, "/api/routing-configs").await;
    let member_id = mine[0]["providers"][0]["id"].as_i64().unwrap();
    assert_eq!(t.call(&t.bob, "DELETE", &format!("/api/routing-configs/providers/{member_id}"), "").await.0, 404);

    let platform_config = r#"{"name":"shared","strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5}"#;
    let (_, shared) = t.call(&t.admin, "POST", "/api/routing-configs", platform_config).await;
    let shared_id = shared["id"].as_i64().unwrap();
    let foreign = format!(r#"{{"routing_config_id":{shared_id},"provider_id":{},"weight":1,"is_active":true}}"#, alice_provider.id);
    assert_eq!(t.call(&t.admin, "POST", "/api/routing-configs/providers", &foreign).await.0, 400);
    t.state.db.set_provider_status(alice_provider.id, "approved", None, false).await.unwrap();
    assert_eq!(t.call(&t.admin, "POST", "/api/routing-configs/providers", &foreign).await.0, 200);
    assert_eq!(t.call(&t.bob, "POST", "/api/routing-configs/providers", &foreign).await.0, 404);

    let (_, candidates) = (0, t.state.config.router.candidate_backends("alice/fast").await);
    assert_eq!(candidates.len(), 1);
    assert_eq!(t.state.config.router.candidate_backends("shared").await.len(), 1);
}

#[tokio::test]
async fn request_log_shows_a_user_only_their_own_requests() {
    let t = tenancy().await;
    let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
    let bob = t.state.db.get_user_by_username("bob").await.unwrap().unwrap();
    let record = |id: &str, user: i64| Record {
        request_id: id.into(),
        started_at_ms: 1,
        endpoint: "/v1/chat/completions".into(),
        model: "m".into(),
        user_id: Some(user),
        provider: Some("plat".into()),
        upstream_url: Some("https://internal.example/v1".into()),
        route: Some("code-high > secret-pool > plat".into()),
        status: "ok".into(),
        attempts: 1,
        ..Default::default()
    };
    let ids = t.state.db.insert_request_logs(&[record("a1", alice.id), record("b1", bob.id)]).await.unwrap();

    let (_, list) = t.get(&t.alice, &format!("/api/request-log?user_id={}", bob.id)).await;
    let seen: Vec<&str> = list.as_array().unwrap().iter().map(|r| r["request_id"].as_str().unwrap()).collect();
    assert_eq!(seen, vec!["a1"]);
    assert!(list[0]["route"].is_null(), "platform routing must be hidden in the list: {list}");
    let (_, admin_list) = t.get(&t.admin, "/api/request-log").await;
    assert_eq!(admin_list[0]["route"], "code-high > secret-pool > plat");
    assert_eq!(t.get(&t.alice, &format!("/api/request-log/{}", ids[1])).await.0, 404);
    assert_eq!(t.get(&t.alice, &format!("/api/request-log/{}/bodies", ids[1])).await.0, 404);
    let (status, own) = t.get(&t.alice, &format!("/api/request-log/{}", ids[0])).await;
    assert_eq!(status, 200);
    assert!(own["upstream_url"].is_null(), "platform upstream must be hidden: {own}");
    let (_, admin_view) = t.get(&t.admin, &format!("/api/request-log/{}", ids[0])).await;
    assert_eq!(admin_view["upstream_url"], "https://internal.example/v1");
}

#[tokio::test]
async fn metrics_cover_only_the_users_own_providers() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let name = t.state.db.get_provider_by_slug(&slug).await.unwrap().unwrap().name;
    let emitter = t.state.metrics_store.emitter().clone();
    emitter.emit_success(&name, "m", None);
    emitter.emit_success("platform-openai", "m", None);
    emitter.emit_success("platform-openai", "m", None);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let (_, alice) = t.get(&t.alice, "/api/metrics").await;
    assert_eq!(alice["total_requests"], 1, "{alice}");
    assert!(alice["recent_events"].as_array().unwrap().iter().all(|e| e["provider"] == name.as_str()));
    let (_, bob) = t.get(&t.bob, "/api/metrics").await;
    assert_eq!(bob["total_requests"], 0);
    assert!(bob["recent_events"].as_array().unwrap().is_empty());
    assert_eq!(t.get(&t.bob, "/api/metrics/health").await.1["provider_count"], 0);
}

#[tokio::test]
async fn pricing_is_scoped_to_the_owners_providers() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    t.state.db.create_provider(NewProvider { name: "plat", slug: "plat", base_url: "http://localhost:1", api_key: None, provider_type: None }).await.unwrap();
    let price = |provider: &str| format!(r#"{{"model_name":"acme/m","provider_slug":"{provider}","is_advertised":true,"is_free":false,"currency":"usd","price_per_1m_input_milli":100}}"#);

    let (status, created) = t.call(&t.alice, "POST", "/api/model-pricing", &price(&slug)).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["currency"], "usd");
    assert_eq!(created["price_per_1m_input_milli"], 100);
    let id = created["id"].as_i64().unwrap();
    assert_eq!(t.call(&t.bob, "POST", "/api/model-pricing", &price(&slug)).await.0, 404);
    assert_eq!(t.call(&t.alice, "POST", "/api/model-pricing", &price("plat")).await.0, 404);
    let platform = r#"{"model_name":"m","is_advertised":true,"is_free":false}"#;
    assert_eq!(t.call(&t.alice, "POST", "/api/model-pricing", platform).await.0, 400);

    assert_eq!(t.get(&t.bob, "/api/model-pricing").await.1.as_array().unwrap().len(), 0);
    assert_eq!(t.call(&t.bob, "PUT", &format!("/api/model-pricing/id/{id}"), r#"{"is_advertised":false}"#).await.0, 404);
    assert_eq!(t.call(&t.bob, "DELETE", &format!("/api/model-pricing/id/{id}"), "").await.0, 404);
    assert_eq!(t.call(&t.alice, "PUT", "/api/model-pricing/acme%2Fm", r#"{"is_free":true}"#).await.0, 403);
    let (status, edited) = t.call(&t.alice, "PUT", &format!("/api/model-pricing/id/{id}"), r#"{"is_free":true,"price_per_1m_input_milli":200}"#).await;
    assert_eq!(status, 200, "{edited}");
    assert_eq!(edited["is_free"], false);
    assert_eq!(edited["price_per_1m_input_milli"], 200);
}

#[tokio::test]
async fn payments_show_a_user_only_their_own_ledger() {
    let t = tenancy().await;
    let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
    let bob = t.state.db.get_user_by_username("bob").await.unwrap().unwrap();
    t.state.db.credit_currency(alice.id, "sats", 1_000, "deposit", "a", 1_000).await.unwrap();
    t.state.db.credit_currency(bob.id, "sats", 2_000, "deposit", "b", 2_000).await.unwrap();

    let (_, balances) = t.get(&t.alice, "/api/payments/balances").await;
    let users: Vec<i64> = balances.as_array().unwrap().iter().map(|b| b["user_id"].as_i64().unwrap()).collect();
    assert_eq!(users, vec![alice.id]);
    let (_, txs) = t.get(&t.alice, "/api/payments/transactions").await;
    assert!(txs.as_array().unwrap().iter().all(|tx| tx["user_id"] == alice.id));
    assert_eq!(txs.as_array().unwrap().len(), 1);
    let (_, everyone) = t.get(&t.admin, "/api/payments/balances").await;
    assert_eq!(everyone.as_array().unwrap().len(), 2);
    t.state.db.credit_currency(alice.id, "eur", 2_500_000, "deposit", "a-eur", 2_500_000).await.unwrap();
    let (_, with_eur) = t.get(&t.alice, "/api/payments/balances").await;
    assert_eq!(with_eur[0]["currencies"][0]["currency"], "eur");
    assert_eq!(with_eur[0]["currencies"][0]["balance_milli"], 2_500_000);
    let credit = format!(r#"{{"user_id":{},"currency":"sats","amount_milli":5}}"#, alice.id);
    assert_eq!(t.call(&t.alice, "POST", "/api/payments/credit", &credit).await.0, 403);
}

#[tokio::test]
async fn metrics_events_never_name_other_callers() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let name = t.state.db.get_provider_by_slug(&slug).await.unwrap().unwrap().name;
    let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
    let caller = |id: i64| crate::metrics::MetricsUser {
        id: Some(id),
        name: Some(format!("caller-{id}")),
        api_key_id: Some(id),
        api_key_name: Some(format!("key-{id}")),
    };
    let emitter = t.state.metrics_store.emitter().clone();
    emitter.emit_success(&name, "m", Some(caller(alice.id + 1000)));
    emitter.emit_success(&name, "m", Some(caller(alice.id)));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let (_, body) = t.get(&t.alice, "/api/metrics").await;
    let text = body["recent_events"].to_string();
    assert!(!text.contains(&format!("caller-{}", alice.id + 1000)), "{text}");
    assert!(!text.contains(&format!("key-{}", alice.id + 1000)), "{text}");
    assert!(text.contains(&format!("caller-{}", alice.id)), "{text}");
}

#[tokio::test]
async fn the_owner_sees_why_their_provider_is_degraded_and_other_tenants_do_not() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let name = t.state.db.get_provider_by_slug(&slug).await.unwrap().unwrap().name;
    let emitter = t.state.metrics_store.emitter().clone();
    let error = crate::ProviderError::ServerError { message: "upstream down".into(), status_code: Some(503) };
    for user in 1..=3 {
        let caller = crate::metrics::MetricsUser { id: Some(user), name: None, api_key_id: None, api_key_name: None };
        emitter.emit_provider_error(&name, "m", &error, Some(caller));
    }

    let (status, alice) = t.get(&t.alice, "/api/metrics/health").await;
    assert_eq!(status, 200, "{alice}");
    let entry = alice["providers"].as_array().unwrap().iter().find(|p| p["provider"] == name.as_str()).unwrap_or_else(|| panic!("{alice}"));
    assert_eq!(entry["health_state"], "degraded");
    assert_eq!(entry["reason"]["cause"], "consecutive_faults");
    assert_eq!(entry["reason"]["provider_faults"], 3);

    let (_, bob) = t.get(&t.bob, "/api/metrics/health").await;
    assert_eq!(bob["provider_count"], 0);
    assert!(!bob.to_string().contains("consecutive_faults"), "{bob}");
}
