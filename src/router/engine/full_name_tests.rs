use super::*;
use crate::config::DefaultPricingConfig;
use crate::db::{FiatPrices, NewModelPricing, NewProvider};
use crate::payments::pricing::PricingResolver;
use crate::request_log::Trace;
use async_trait::async_trait;
use std::sync::atomic::AtomicBool;

pub(super) struct Listed {
    pub(super) name: String,
    models: Vec<String>,
    calls: AtomicUsize,
    pub(super) failing: AtomicBool,
}

impl Listed {
    pub(super) fn new(name: &str, models: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            models: models.iter().map(|m| m.to_string()).collect(),
            calls: AtomicUsize::new(0),
            failing: AtomicBool::new(false),
        })
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for Listed {
    fn name(&self) -> &str {
        &self.name
    }
    fn slug(&self) -> &str {
        &self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(self
            .models
            .iter()
            .map(|id| crate::Model { id: id.clone(), object: "model".into(), created: 0, owned_by: "x".into() })
            .collect())
    }
    async fn chat_completions(&self, _request: &ChatRequest) -> Result<ChatCompletionResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.failing.load(Ordering::SeqCst) {
            return Err(ProviderError::ServerError { message: "down".into(), status_code: Some(503) });
        }
        Ok(serde_json::from_str(
            r#"{"id":"c","object":"chat.completion","created":1,"model":"m",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}"#,
        )
        .unwrap())
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        Err(ProviderError::Unsupported("no stream".into()))
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

pub(super) fn chat(model: &str, trace: &Trace) -> ChatRequest {
    let request: ChatRequest =
        serde_json::from_str(&format!(r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#))
            .unwrap();
    request.with_trace(trace.clone())
}

pub(super) struct World {
    pub(super) router: Router,
    pub(super) db: Arc<Database>,
    pub(super) metrics: MetricsStore,
}

pub(super) async fn world() -> World {
    let db = Arc::new(Database::test().await);
    let metrics = MetricsStore::new(100);
    let router = Router::new(metrics.clone(), db.clone());
    World { router, db, metrics }
}

pub(super) async fn add_owned(w: &World, owner: Option<i64>, backend: &Arc<Listed>, model: &str, price_milli: Option<i64>) -> i64 {
    let new = NewProvider {
        name: &backend.name,
        slug: &backend.name,
        base_url: "https://example.com/v1",
        api_key: None,
        provider_type: None,
    };
    let record = match owner {
        Some(id) => {
            let created = w.db.create_owned_provider(id, new).await.unwrap();
            w.db.set_provider_status(created.id, crate::db::APPROVED, None, false).await.unwrap()
        }
        None => w.db.create_provider(new).await.unwrap(),
    };
    if let Some(price) = price_milli {
        w.db
            .create_model_pricing(NewModelPricing {
                model_name: model,
                is_advertised: true,
                is_free: false,
                price_per_1m_input_sats: None,
                price_per_1m_output_sats: None,
                price_per_1m_cache_read_sats: None,
                price_per_1m_cache_write_sats: None,
                price_per_request_sats: None,
                context_window: None,
                max_output_tokens: None,
                owner_user_id: owner,
                provider_id: Some(record.id),
                currency: "usd",
                fiat: FiatPrices { input: Some(price), output: Some(price), ..Default::default() },
            })
            .await
            .unwrap();
    }
    w.router.add_provider(backend.clone()).await;
    record.id
}

#[tokio::test]
async fn full_name_requests_alternate_between_user_providers_and_bill_each_at_its_own_price() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let bob = w.db.add_test_user("bob", false).await;
    let a = Listed::new("alice-p", &["Qwen/Qwen3.8-27B"]);
    let b = Listed::new("bob-p", &["qwen/qwen3.8-27b"]);
    add_owned(&w, Some(alice.id), &a, "qwen/qwen3.8-27b", Some(1_000_000)).await;
    add_owned(&w, Some(bob.id), &b, "qwen/qwen3.8-27b", Some(3_000_000)).await;
    let resolver = PricingResolver::new(w.db.clone(), DefaultPricingConfig::default());

    let mut served = Vec::new();
    for _ in 0..4 {
        let trace = Trace::new();
        w.router.chat_completions(&chat("QWEN/qwen3.8-27b", &trace), None).await.unwrap();
        served.push(trace.provider().unwrap());
    }
    assert_eq!((a.calls(), b.calls()), (2, 2));
    assert_ne!(served[0], served[1]);
    assert_eq!(served[0], served[2]);

    let cheap = resolver.resolve_served("qwen/qwen3.8-27b", Some("alice-p")).await;
    let dear = resolver.resolve_served("qwen/qwen3.8-27b", Some("bob-p")).await;
    assert_eq!((cheap.price_per_1m_input_sats, cheap.owner_user_id, cheap.currency.as_str()), (1_000_000, Some(alice.id), "usd"));
    assert_eq!((dear.price_per_1m_input_sats, dear.owner_user_id), (3_000_000, Some(bob.id)));
}

#[tokio::test]
async fn platform_and_user_providers_share_one_pool() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let user = Listed::new("alice-p", &["qwen/qwen3.8-27b"]);
    let platform = Listed::new("platform-p", &["qwen/qwen3.8-27b"]);
    add_owned(&w, Some(alice.id), &user, "qwen/qwen3.8-27b", Some(10)).await;
    add_owned(&w, None, &platform, "qwen/qwen3.8-27b", None).await;
    for _ in 0..4 {
        w.router.chat_completions(&chat("qwen/qwen3.8-27b", &Trace::new()), None).await.unwrap();
    }
    assert_eq!((user.calls(), platform.calls()), (2, 2));
}

#[tokio::test]
async fn short_names_never_reach_user_providers() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let user = Listed::new("alice-p", &["qwen3.8-27b"]);
    add_owned(&w, Some(alice.id), &user, "qwen3.8-27b", Some(10)).await;
    let result = w.router.chat_completions(&chat("qwen3.8-27b", &Trace::new()), None).await;
    assert!(matches!(result, Err(RouterError::NoAvailableProvider)));
    assert_eq!(user.calls(), 0);
}

#[tokio::test]
async fn an_unhealthy_user_provider_is_skipped() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let bob = w.db.add_test_user("bob", false).await;
    let sick = Listed::new("alice-p", &["acme/m-1"]);
    let well = Listed::new("bob-p", &["acme/m-1"]);
    add_owned(&w, Some(alice.id), &sick, "acme/m-1", Some(10)).await;
    add_owned(&w, Some(bob.id), &well, "acme/m-1", Some(10)).await;
    sick.failing.store(true, Ordering::SeqCst);
    w.metrics
        .record(crate::metrics::ProviderMetrics {
            provider: "alice-p".to_string(),
            model: "acme/m-1".to_string(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            event: crate::metrics::MetricsEvent::Failure(crate::metrics::FailureDetails {
                error_type: crate::metrics::ErrorType::RateLimit,
                error_code: None,
                error_message: "429".to_string(),
                retry_after_ms: Some(60_000),
                status_code: Some(429),
            }),
            user: None,
        })
        .await;
    assert!(w.metrics.time_until_retry("alice-p").await.as_millis() > 0);

    for _ in 0..6 {
        w.router.chat_completions(&chat("acme/m-1", &Trace::new()), None).await.unwrap();
    }
    assert_eq!((sick.calls(), well.calls()), (0, 6));
}

#[tokio::test]
async fn an_unpriced_user_provider_stays_out_of_the_pool() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let user = Listed::new("alice-p", &["acme/m-1"]);
    add_owned(&w, Some(alice.id), &user, "acme/m-1", None).await;
    let result = w.router.chat_completions(&chat("acme/m-1", &Trace::new()), None).await;
    assert!(matches!(result, Err(RouterError::NoAvailableProvider)));
}

#[tokio::test]
async fn an_exact_routing_config_wins_over_the_pool() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let user = Listed::new("alice-p", &["acme/m-1"]);
    let configured = Listed::new("configured", &["acme/m-1"]);
    add_owned(&w, Some(alice.id), &user, "acme/m-1", Some(10)).await;
    w.router.add_provider(configured.clone()).await;
    w.router.register_route("acme/m-1", vec![configured.clone()]).await;
    for _ in 0..3 {
        w.router.chat_completions(&chat("acme/m-1", &Trace::new()), None).await.unwrap();
    }
    assert_eq!((configured.calls(), user.calls()), (3, 0));
}

#[tokio::test]
async fn slug_prefixed_models_still_route_directly() {
    let w = world().await;
    let direct = Listed::new("fast", &["m"]);
    w.router.add_provider(direct.clone()).await;
    w.router.chat_completions(&chat("fast/m", &Trace::new()), None).await.unwrap();
    assert_eq!(direct.calls(), 1);
}

fn as_user(id: i64) -> Option<crate::metrics::MetricsUser> {
    Some(crate::metrics::MetricsUser { id: Some(id), name: None, api_key_id: None, api_key_name: None })
}

#[tokio::test]
async fn a_pending_provider_serves_its_owner_but_not_other_users() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let bob = w.db.add_test_user("bob", false).await;
    let pending = Listed::new("alice-p", &["acme/m-1"]);
    let record = w
        .db
        .create_owned_provider(
            alice.id,
            NewProvider { name: "alice-p", slug: "alice-p", base_url: "https://example.com/v1", api_key: None, provider_type: None },
        )
        .await
        .unwrap();
    assert_eq!(record.status, crate::db::PENDING);
    w.db.create_model_pricing(NewModelPricing {
        model_name: "acme/m-1",
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: None,
        price_per_1m_output_sats: None,
        price_per_1m_cache_read_sats: None,
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: None,
        context_window: None,
        max_output_tokens: None,
        owner_user_id: Some(alice.id),
        provider_id: Some(record.id),
        currency: "usd",
        fiat: FiatPrices { input: Some(10), output: Some(10), ..Default::default() },
    })
    .await
    .unwrap();
    w.router.add_provider(pending.clone()).await;

    let denied = w.router.chat_completions(&chat("acme/m-1", &Trace::new()), as_user(bob.id)).await;
    assert!(matches!(denied, Err(RouterError::NoAvailableProvider)));
    let anonymous = w.router.chat_completions(&chat("acme/m-1", &Trace::new()), None).await;
    assert!(matches!(anonymous, Err(RouterError::NoAvailableProvider)));
    assert_eq!(pending.calls(), 0);

    w.router.chat_completions(&chat("acme/m-1", &Trace::new()), as_user(alice.id)).await.unwrap();
    assert_eq!(pending.calls(), 1);

    w.db.set_provider_status(record.id, crate::db::APPROVED, None, false).await.unwrap();
    w.router.refresh_full_name_index().await;
    w.router.chat_completions(&chat("acme/m-1", &Trace::new()), as_user(bob.id)).await.unwrap();
    assert_eq!(pending.calls(), 2);

    w.db.set_provider_status(record.id, crate::db::SUSPENDED, Some("failed"), false).await.unwrap();
    w.router.refresh_full_name_index().await;
    let suspended = w.router.chat_completions(&chat("acme/m-1", &Trace::new()), as_user(alice.id)).await;
    assert!(matches!(suspended, Err(RouterError::NoAvailableProvider)));
}
