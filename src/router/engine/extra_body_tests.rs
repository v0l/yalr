use super::*;
use crate::db::{NewProvider, NewRoutingConfig, NewRoutingConfigChild, NewRoutingConfigProvider, ProviderType};
use crate::providers::OpenAiProvider;
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPLY: &str = r#"{"id":"c","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#;

fn map(value: Value) -> crate::db::ExtraBody {
    value.as_object().unwrap().clone()
}

struct Setup {
    router: Router,
    upstream: MockServer,
}

async fn setup(provider_body: Value, entry_body: Option<Value>) -> Setup {
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REPLY))
        .mount(&upstream)
        .await;
    let db = Arc::new(Database::test().await);
    let record = db
        .create_provider(NewProvider {
            name: "up",
            slug: "up",
            base_url: &format!("{}/v1", upstream.uri()),
            api_key: Some("k"),
            provider_type: Some(ProviderType::OpenAi),
        })
        .await
        .unwrap();
    db.set_provider_extra_body(record.id, Some(&map(provider_body))).await.unwrap();
    let config = db
        .create_routing_config(NewRoutingConfig {
            name: "m".into(),
            strategy: "round_robin".into(),
            health_check_enabled: false,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        })
        .await
        .unwrap();
    let entry = db
        .create_routing_config_provider(NewRoutingConfigProvider {
            routing_config_id: config.id,
            provider_id: record.id,
            model: None,
            weight: 1,
            is_active: true,
        })
        .await
        .unwrap();
    if let Some(body) = entry_body {
        db.set_routing_config_provider_extra_body(entry.id, Some(&map(body))).await.unwrap();
    }
    let router = Router::new(MetricsStore::new(100), db);
    router.reload_config().await.unwrap();
    Setup { router, upstream }
}

async fn last_chat_body(upstream: &MockServer) -> Value {
    let received = upstream.received_requests().await.unwrap();
    let chat = received.iter().rev().find(|r| r.method.to_string() == "POST").unwrap();
    serde_json::from_slice(&chat.body).unwrap()
}

async fn send(setup: &Setup, extra: &str) -> Value {
    let request: ChatRequest = serde_json::from_str(&format!(
        r#"{{"model":"m","messages":[{{"role":"user","content":"hi"}}]{extra}}}"#
    ))
    .unwrap();
    setup.router.chat_completions(&request, None).await.unwrap();
    last_chat_body(&setup.upstream).await
}

#[tokio::test]
async fn provider_extra_body_reaches_the_upstream() {
    let s = setup(json!({"provider": {"order": ["x"], "allow_fallbacks": false}}), None).await;
    let sent = send(&s, "").await;
    assert_eq!(sent["provider"], json!({"order": ["x"], "allow_fallbacks": false}));
}

#[tokio::test]
async fn routing_entry_extra_body_overrides_the_providers_for_the_same_key() {
    let s = setup(
        json!({"provider": {"order": ["x"]}, "top_k": 3}),
        Some(json!({"provider": {"order": ["y"]}})),
    )
    .await;
    let sent = send(&s, "").await;
    assert_eq!(sent["provider"], json!({"order": ["y"]}));
    assert_eq!(sent["top_k"], 3);
}

#[tokio::test]
async fn a_client_provider_field_replaces_the_configured_one() {
    let s = setup(
        json!({"provider": {"order": ["x"], "allow_fallbacks": false}}),
        Some(json!({"provider": {"order": ["y"]}})),
    )
    .await;
    let sent = send(&s, r#","provider":{"only":["z"]}"#).await;
    assert_eq!(sent["provider"], json!({"only": ["z"]}));
}

#[tokio::test]
async fn nested_config_entry_extra_body_is_applied_by_the_outermost_last() {
    let s = setup(json!({"provider": {"order": ["leaf"]}, "a": "provider"}), Some(json!({"a": "entry"}))).await;
    let db = s.router.db.clone();
    let inner = db.get_first_routing_config().await.unwrap().unwrap();
    let outer = db
        .create_routing_config(NewRoutingConfig {
            name: "outer".into(),
            strategy: "round_robin".into(),
            health_check_enabled: false,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        })
        .await
        .unwrap();
    let link = db
        .create_routing_config_child(NewRoutingConfigChild {
            routing_config_id: outer.id,
            child_config_id: inner.id,
            model: None,
            weight: 1,
            is_active: true,
        })
        .await
        .unwrap();
    db.set_routing_config_provider_extra_body(link.id, Some(&map(json!({"a": "outer"})))).await.unwrap();
    s.router.reload_config().await.unwrap();

    let request: ChatRequest =
        serde_json::from_str(r#"{"model":"outer","messages":[{"role":"user","content":"hi"}]}"#).unwrap();
    s.router.chat_completions(&request, None).await.unwrap();
    let sent = last_chat_body(&s.upstream).await;
    assert_eq!(sent["a"], "outer");
    assert_eq!(sent["provider"], json!({"order": ["leaf"]}));
}
