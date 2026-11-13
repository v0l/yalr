use crate::api::server::create_test_app;
use crate::db::{Database, NewApiKey};
use crate::metrics::{HealthState, MetricsStore};
use crate::providers::{ChatRequest, Provider, ProviderError};
use crate::router::transient::TransientRetry;
use crate::state::AppState;
use axum::body::Body;
use axum::http::Request;
use futures::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower::util::ServiceExt;

const KEY: &str = "sk-retry-test";
const MODEL: &str = "code-high";

fn status(code: u16) -> ProviderError {
    ProviderError::ServerError { message: String::new(), status_code: Some(code) }
}

fn limited(seconds: u64) -> ProviderError {
    ProviderError::RateLimit { retry_after_ms: seconds * 1000, message: "slow down".into() }
}

struct Scripted {
    name: String,
    script: Vec<Result<(), ProviderError>>,
    calls: AtomicUsize,
    chunk_first: bool,
}

impl Scripted {
    fn new(name: &str, script: Vec<Result<(), ProviderError>>) -> Arc<Self> {
        Arc::new(Self { name: name.into(), script, calls: AtomicUsize::new(0), chunk_first: false })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn step(&self) -> Result<(), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.script[n.min(self.script.len() - 1)].clone()
    }
}

fn chunk() -> crate::StreamingChunk {
    serde_json::from_str(
        r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
    )
    .unwrap()
}

#[async_trait::async_trait]
impl Provider for Scripted {
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
        self.step()?;
        Ok(crate::providers::ChatResponse::from_value(serde_json::json!({
            "id": "r", "object": "chat.completion", "created": 1, "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
        }))
        .unwrap())
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>, ProviderError> {
        if self.chunk_first {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let items = vec![Ok(chunk()), Err(status(503))];
            return Ok(Box::pin(futures::stream::iter(items)));
        }
        match self.step() {
            Err(e) => Err(e),
            Ok(()) => Ok(Box::pin(futures::stream::iter(vec![Ok(chunk())]))),
        }
    }
    async fn image_generations(
        &self,
        _request: &crate::providers::image::ImageGenerationRequest,
    ) -> Result<crate::providers::image::ImageResponse, ProviderError> {
        self.step()?;
        Ok(crate::providers::image::ImageResponse {
            content_type: "application/json".into(),
            stream: Box::pin(futures::stream::iter(vec![Ok(bytes::Bytes::from_static(b"{}"))])),
        })
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

struct Harness {
    state: Arc<AppState>,
    metrics: Arc<MetricsStore>,
}

fn fast_retry() -> TransientRetry {
    TransientRetry { rounds: 3, budget: Duration::from_secs(15), base: Duration::from_millis(20) }
}

async fn harness(providers: Vec<Arc<Scripted>>) -> Harness {
    let db = Arc::new(Database::test().await);
    let metrics = Arc::new(MetricsStore::new(100));
    let router = Arc::new(
        crate::router::engine::Router::new((*metrics).clone(), db.clone()).with_transient_retry(fast_retry()),
    );
    let mut routed: Vec<Arc<dyn Provider>> = Vec::new();
    for provider in providers {
        router.add_provider(provider.clone()).await;
        routed.push(provider);
    }
    router.register_route(MODEL, routed).await;
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
        metrics_emitter: metrics.emitter().clone(),
        metrics_store: metrics.clone(),
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
    Harness { state, metrics }
}

async fn chat(state: &Arc<AppState>) -> (axum::http::StatusCode, Option<String>) {
    let request = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)))
        .unwrap();
    let response = create_test_app(state.clone()).await.oneshot(request).await.unwrap();
    let retry_after = response.headers().get("retry-after").map(|v| v.to_str().unwrap().to_string());
    (response.status(), retry_after)
}

fn chat_request() -> ChatRequest {
    serde_json::from_str(&format!(r#"{{"model":"{MODEL}","messages":[{{"role":"user","content":"hi"}}]}}"#)).unwrap()
}

#[tokio::test]
async fn a_single_provider_503_then_ok_is_retried_and_served() {
    let only = Scripted::new("max", vec![Err(status(503)), Ok(())]);
    let h = harness(vec![only.clone()]).await;
    let (code, _) = chat(&h.state).await;
    assert_eq!(code, 200);
    assert_eq!(only.calls(), 2);
    assert_eq!(h.metrics.emitter().get_transient_retries(), (1, 1));
    assert_eq!(h.metrics.get_provider_health("max").await, HealthState::Healthy);
}

#[tokio::test]
async fn a_single_provider_503_every_time_stops_after_the_configured_rounds() {
    let only = Scripted::new("max", vec![Err(status(503))]);
    let h = harness(vec![only.clone()]).await;
    let started = Instant::now();
    let (code, retry_after) = chat(&h.state).await;
    assert_eq!(code, 503);
    assert_eq!(retry_after.as_deref(), Some("30"));
    assert_eq!(only.calls(), 4);
    assert!(started.elapsed() < Duration::from_secs(15));
    assert_eq!(h.metrics.emitter().get_transient_retries(), (3, 0));
}

#[tokio::test]
async fn the_budget_stops_the_rounds_early() {
    let only = Scripted::new("max", vec![Err(status(503))]);
    let h = harness(vec![only.clone()]).await;
    let router = Arc::new(
        crate::router::engine::Router::new((*h.metrics).clone(), h.state.db.clone()).with_transient_retry(
            TransientRetry { rounds: 3, budget: Duration::from_millis(600), base: Duration::from_millis(150) },
        ),
    );
    router.add_provider(only.clone()).await;
    router.register_route(MODEL, vec![only.clone()]).await;
    let err = router.chat_completions(&chat_request(), None).await.unwrap_err();
    assert_eq!(only.calls(), 2, "{err}");
}

#[tokio::test]
async fn a_healthy_second_provider_serves_without_an_extra_round() {
    let a = Scripted::new("a", vec![Err(status(503))]);
    let b = Scripted::new("b", vec![Ok(())]);
    let h = harness(vec![a.clone(), b.clone()]).await;
    let (code, _) = chat(&h.state).await;
    assert_eq!(code, 200);
    assert_eq!((a.calls(), b.calls()), (1, 1));
    assert_eq!(h.metrics.emitter().get_transient_retries(), (0, 0));
}

#[tokio::test]
async fn a_stream_that_errors_after_a_chunk_is_not_retried() {
    let mut inner = Scripted { name: "max".into(), script: vec![Ok(())], calls: AtomicUsize::new(0), chunk_first: true };
    inner.chunk_first = true;
    let only = Arc::new(inner);
    let h = harness(vec![only.clone()]).await;
    let mut stream = h.state.config.router.chat_completions_stream(&chat_request(), None).await.unwrap();
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    assert_eq!(items.len(), 2);
    assert!(items[0].is_ok());
    assert!(items[1].is_err());
    assert_eq!(only.calls(), 1);
}

#[tokio::test]
async fn a_stream_503_before_any_chunk_is_retried() {
    let only = Scripted::new("max", vec![Err(status(503)), Ok(())]);
    let h = harness(vec![only.clone()]).await;
    let mut stream = h.state.config.router.chat_completions_stream(&chat_request(), None).await.unwrap();
    let first = stream.next().await.unwrap();
    assert!(first.is_ok());
    assert_eq!(only.calls(), 2);
}

#[tokio::test]
async fn client_errors_are_not_retried() {
    for code in [400, 402] {
        let only = Scripted::new("max", vec![Err(status(code)), Ok(())]);
        let h = harness(vec![only.clone()]).await;
        let (answered, _) = chat(&h.state).await;
        assert_ne!(answered, 200, "{code}");
        assert_eq!(only.calls(), 1, "{code}");
    }
}

#[tokio::test]
async fn a_rate_limited_provider_drops_out_of_the_rounds() {
    let a = Scripted::new("a", vec![Err(status(503))]);
    let b = Scripted::new("b", vec![Err(limited(120))]);
    let h = harness(vec![a.clone(), b.clone()]).await;
    let (code, _) = chat(&h.state).await;
    assert_eq!(code, 503);
    assert_eq!(a.calls(), 4);
    assert_eq!(b.calls(), 1);
}

#[tokio::test]
async fn two_rate_limits_report_the_earliest_reset_without_retrying() {
    let a = Scripted::new("a", vec![Err(limited(108_000))]);
    let b = Scripted::new("b", vec![Err(limited(120))]);
    let h = harness(vec![a.clone(), b.clone()]).await;
    let (code, retry_after) = chat(&h.state).await;
    assert_eq!(code, 429);
    assert_eq!(retry_after.as_deref(), Some("120"));
    assert_eq!((a.calls(), b.calls()), (1, 1));
}

#[tokio::test]
async fn an_image_request_is_retried_after_a_503() {
    let only = Scripted::new("max", vec![Err(status(503)), Ok(())]);
    let h = harness(vec![only.clone()]).await;
    let request: crate::providers::image::ImageGenerationRequest =
        serde_json::from_str(&format!(r#"{{"model":"{MODEL}","prompt":"a cat"}}"#)).unwrap();
    let response = h.state.config.router.image_generations(&request, None).await.unwrap();
    assert_eq!(response.content_type, "application/json");
    assert_eq!(only.calls(), 2);
    assert_eq!(h.metrics.emitter().get_transient_retries(), (1, 1));
    assert_eq!(h.metrics.get_recent_failures("max").await, 0);
}

#[tokio::test]
async fn a_failure_that_a_retry_cured_leaves_no_trace_in_health() {
    let only = Scripted::new("max", vec![Err(status(503)), Ok(())]);
    let h = harness(vec![only.clone()]).await;
    assert_eq!(chat(&h.state).await.0, 200);
    assert_eq!(h.metrics.get_recent_failures("max").await, 0);
}

#[tokio::test]
async fn a_request_that_finally_fails_records_its_first_round_failure() {
    let only = Scripted::new("max", vec![Err(status(503))]);
    let h = harness(vec![only.clone()]).await;
    assert_eq!(chat(&h.state).await.0, 503);
    assert_eq!(h.metrics.get_recent_failures("max").await, 1);
}
