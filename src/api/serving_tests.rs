use crate::api::server::create_test_app;
use crate::db::{Database, NewApiKey};
use crate::metrics::MetricsStore;
use crate::providers::provider_trait::QuotaSnapshot;
use crate::providers::OpenAiProvider;
use crate::state::AppState;
use axum::body::Body;
use axum::http::Request;
use std::sync::Arc;
use tower::util::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-serving-test";
const MODEL: &str = "claude-sonnet-4-5";

async fn harness() -> (Arc<AppState>, MockServer) {
    let upstream = MockServer::start().await;
    let db = Arc::new(Database::test().await);
    let metrics_store = Arc::new(MetricsStore::new(100));
    let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
    let provider = Arc::new(OpenAiProvider::new("up", Some("up"), &format!("{}/v1", upstream.uri()), Some("k")));
    router.add_provider(provider.clone()).await;
    router.register_route(MODEL, vec![provider]).await;
    metrics_store.emitter().emit_quota(
        "up",
        vec![
            QuotaSnapshot {
                window: Some("5h".into()),
                used_pct: Some(12.5),
                status: Some("allowed".into()),
                resets_at: Some(1_794_321_000_000),
                ..Default::default()
            },
            QuotaSnapshot {
                window: Some("7d".into()),
                used_pct: Some(80.0),
                status: Some("allowed_warning".into()),
                resets_at: Some(1_794_628_800_000),
                ..Default::default()
            },
        ],
        None,
    );
    let state = Arc::new(AppState {
        config: crate::config::AppConfig {
            db: db.clone(),
            router,
            auth_config: Default::default(),
            payments_config: None,
            discovery_config: None,
            admin_ui_path: String::new(),
            host: String::new(),
            port: 0,
        },
        metrics_emitter: metrics_store.emitter().clone(),
        metrics_store,
        session_store: Arc::new(crate::auth::admin::SessionStore::new()),
        db: db.clone(),
        payments_state: None,
        oauth_pending: Default::default(),
        model_cache: Default::default(),
        modality_cache: Default::default(),
        limits_cache: Default::default(),
    });
    let user = db.add_test_user("buyer", false).await;
    db.create_api_key(NewApiKey {
        key_hash: &crate::auth::api_key::hash_api_key(KEY),
        name: "main",
        user_id: user.id,
        last_four: "test",
        expires_at: None,
    })
    .await
    .unwrap();
    (state, upstream)
}

async fn post(state: &Arc<AppState>, uri: &str, body: &str) -> axum::response::Response {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", format!("Bearer {KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    create_test_app(state.clone()).await.oneshot(request).await.unwrap()
}

fn assert_headers(response: &axum::response::Response) {
    let header = |name: &str| response.headers().get(name).unwrap().to_str().unwrap().to_string();
    assert_eq!(header("x-yalr-quota-used-pct"), "80");
    assert_eq!(header("x-yalr-quota-window"), "7d");
    assert_eq!(header("x-yalr-quota-resets-at"), "1794628800000");
    assert_eq!(header("x-yalr-cost-usd"), "0.010500");
}

fn assert_report(usage: &serde_json::Value) {
    assert!((usage["cost_usd"].as_f64().unwrap() - 0.0105).abs() < 1e-9, "{usage}");
    assert_eq!(usage["quota"]["window"], "7d");
    assert_eq!(usage["quota"]["used_pct"], 80.0);
    assert_eq!(usage["quota"]["resets_at"], 1_794_628_800_000i64);
}

#[tokio::test]
async fn chat_response_reports_cost_and_quota() {
    let (state, upstream) = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1", "object": "chat.completion", "created": 1, "model": MODEL,
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 500, "total_tokens": 1500}
        })))
        .mount(&upstream)
        .await;
    let response = post(&state, "/v1/chat/completions", &format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)).await;
    assert_eq!(response.status(), 200);
    assert_headers(&response);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_report(&body["usage"]);
    assert_eq!(body["usage"]["prompt_tokens"], 1000);
}

#[tokio::test]
async fn openrouter_reported_cost_wins_over_the_table() {
    let (state, upstream) = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1", "object": "chat.completion", "created": 1, "model": MODEL,
            "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "hi"}}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 500, "total_tokens": 1500, "cost": 0.5}
        })))
        .mount(&upstream)
        .await;
    let response = post(&state, "/v1/chat/completions", &format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)).await;
    assert_eq!(response.headers()["x-yalr-cost-usd"], "0.500000");
}

#[tokio::test]
async fn streamed_chat_reports_cost_and_quota_in_the_usage_chunk() {
    let (state, upstream) = harness().await;
    let sse = format!(
        "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"{MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"hi\"}},\"finish_reason\":null}}]}}\n\n\
         data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"{MODEL}\",\"choices\":[],\"usage\":{{\"prompt_tokens\":1000,\"completion_tokens\":500,\"total_tokens\":1500}}}}\n\n\
         data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&upstream)
        .await;
    let body = format!(r#"{{"model":"{MODEL}","stream":true,"stream_options":{{"include_usage":true}},"messages":[{{"role":"user","content":"hi"}}]}}"#);
    let response = post(&state, "/v1/chat/completions", &body).await;
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let usage_chunk = text
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .filter_map(|d| serde_json::from_str::<serde_json::Value>(d.trim()).ok())
        .find(|v| v.get("usage").is_some_and(|u| !u.is_null()))
        .expect("a usage chunk");
    assert_report(&usage_chunk["usage"]);
}

#[tokio::test]
async fn responses_api_reports_cost_and_quota() {
    let (state, upstream) = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "resp_1", "object": "response", "created_at": 0, "model": MODEL, "status": "completed",
            "output": [], "usage": {"input_tokens": 1000, "output_tokens": 500, "total_tokens": 1500,
            "input_tokens_details": {"cached_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 0}}
        })))
        .mount(&upstream)
        .await;
    let response = post(&state, "/v1/responses", &format!(r#"{{"model":"{MODEL}","input":"hi"}}"#)).await;
    assert_eq!(response.status(), 200);
    assert_headers(&response);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_report(&body["usage"]);
}

#[tokio::test]
async fn a_rate_limited_pool_answers_429_with_next_available_at() {
    let (state, upstream) = harness().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"message": "slow down", "code": 429, "metadata": {"retry_after_seconds": 1}}
        })))
        .mount(&upstream)
        .await;
    let response = post(&state, "/v1/chat/completions", &format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)).await;
    assert_eq!(response.status(), 429);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let at = body["error"]["next_available_at"].as_i64().unwrap();
    assert!(at > chrono::Utc::now().timestamp_millis());
}
