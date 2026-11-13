use crate::api::server::create_test_app;
use crate::db::{Database, NewApiKey};
use crate::metrics::MetricsStore;
use crate::providers::provider_trait::QuotaSnapshot;
use crate::providers::{ChatRequest, Provider, ProviderError};
use crate::state::AppState;
use axum::body::Body;
use axum::http::Request;
use std::sync::Arc;
use tower::util::ServiceExt;

const KEY: &str = "sk-failover-test";
const MODEL: &str = "claude-code";
const THIRTY_HOURS: u64 = 108_000;

enum Upstream {
    Outage,
    Limited(u64),
}

struct Fake {
    name: String,
    upstream: Upstream,
}

impl Fake {
    fn error(&self) -> ProviderError {
        match self.upstream {
            Upstream::Outage => ProviderError::ServerError {
                message: "credential validation failed".into(),
                status_code: Some(503),
            },
            Upstream::Limited(seconds) => ProviderError::RateLimit {
                retry_after_ms: seconds * 1000,
                message: "slow down".into(),
            },
        }
    }
}

#[async_trait::async_trait]
impl Provider for Fake {
    fn name(&self) -> &str {
        &self.name
    }
    fn slug(&self) -> &str {
        &self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![])
    }
    async fn chat_completions(&self, _request: &ChatRequest) -> Result<crate::providers::ChatResponse, ProviderError> {
        Err(self.error())
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>, ProviderError> {
        Err(self.error())
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

async fn harness(first: Upstream, second: Upstream, quotas: Vec<QuotaSnapshot>) -> (Arc<AppState>, ()) {
    let db = Arc::new(Database::test().await);
    let metrics_store = Arc::new(MetricsStore::new(100));
    let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
    for (i, upstream) in [first, second].into_iter().enumerate() {
        let provider = Arc::new(Fake { name: format!("max-{i}"), upstream });
        router.add_provider(provider.clone()).await;
        providers.push(provider);
    }
    router.register_route(MODEL, providers).await;
    if !quotas.is_empty() {
        for name in ["max-0", "max-1"] {
            metrics_store.emitter().emit_quota(name, quotas.clone(), None);
        }
    }
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
    (state, ())
}

async fn chat(state: &Arc<AppState>) -> (axum::http::StatusCode, Option<String>, serde_json::Value) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)))
        .unwrap();
    let response = create_test_app(state.clone()).await.oneshot(request).await.unwrap();
    let status = response.status();
    let retry_after = response.headers().get("retry-after").map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, retry_after, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn assert_outage(first: Upstream, second: Upstream) {
    let (state, _servers) = harness(first, second, vec![]).await;
    let (status, retry_after, body) = chat(&state).await;
    assert_eq!(status, 503, "{body}");
    assert_eq!(retry_after.as_deref(), Some("30"));
    assert_eq!(body["error"]["type"], "upstream_unavailable");
    assert!(body["error"]["message"].as_str().unwrap().contains("outage"));
}

#[tokio::test]
async fn an_outage_then_a_far_rate_limit_answers_503() {
    assert_outage(Upstream::Outage, Upstream::Limited(THIRTY_HOURS)).await;
}

#[tokio::test]
async fn a_far_rate_limit_then_an_outage_answers_503() {
    assert_outage(Upstream::Limited(THIRTY_HOURS), Upstream::Outage).await;
}

#[tokio::test]
async fn two_rate_limits_answer_the_earliest_reset() {
    let (state, _servers) = harness(Upstream::Limited(THIRTY_HOURS), Upstream::Limited(120), vec![]).await;
    let (status, retry_after, body) = chat(&state).await;
    assert_eq!(status, 429, "{body}");
    assert_eq!(retry_after.as_deref(), Some("120"));
}

#[tokio::test]
async fn a_rate_limit_is_capped_at_the_model_next_available_time() {
    let quotas = vec![QuotaSnapshot {
        window: Some("7d".into()),
        used_pct: Some(100.0),
        status: Some("rejected".into()),
        resets_at: Some(chrono::Utc::now().timestamp_millis() + 3_600_000),
        ..Default::default()
    }];
    let (state, _servers) = harness(Upstream::Limited(THIRTY_HOURS), Upstream::Limited(THIRTY_HOURS), quotas).await;
    let (status, retry_after, body) = chat(&state).await;
    assert_eq!(status, 429, "{body}");
    let seconds: u64 = retry_after.unwrap().parse().unwrap();
    assert!((3_500..=3_600).contains(&seconds), "{seconds}");
}
