use super::*;
use crate::request_log::Trace;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

struct Backend {
    name: String,
    calls: AtomicUsize,
    failing: AtomicBool,
    models: Mutex<Vec<String>>,
}

impl Backend {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            calls: AtomicUsize::new(0),
            failing: AtomicBool::new(false),
            models: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for Backend {
    fn name(&self) -> &str {
        &self.name
    }
    fn slug(&self) -> &str {
        &self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![])
    }
    async fn chat_completions(&self, request: &ChatRequest) -> Result<ChatCompletionResponse, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.models.lock().unwrap().push(request.model.clone());
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

fn entry(backend: &Arc<Backend>, weight: i32) -> ProviderEntry {
    ProviderEntry { provider: backend.clone(), model_override: None, weight, extra_body: Default::default() }
}

fn body(value: serde_json::Value) -> crate::db::ExtraBody {
    value.as_object().unwrap().clone()
}

fn entry_with_body(backend: &Arc<Backend>, extra_body: crate::db::ExtraBody) -> ProviderEntry {
    ProviderEntry { provider: backend.clone(), model_override: None, weight: 1, extra_body }
}

fn child(config: &str, weight: i32, model_override: Option<&str>) -> ChildEntry {
    ChildEntry {
        config: config.to_string(),
        model_override: model_override.map(str::to_string),
        weight,
        extra_body: Default::default(),
    }
}

fn child_with_body(config: &str, extra_body: crate::db::ExtraBody) -> ChildEntry {
    ChildEntry { config: config.to_string(), model_override: None, weight: 1, extra_body }
}

async fn router_with(tables: Vec<(&str, RoutingTable)>) -> Router {
    let db = Arc::new(Database::test().await);
    let router = Router::new(MetricsStore::new(100), db);
    let mut map = router.routing_tables.write().await;
    for (name, table) in tables {
        map.insert(name.to_string(), table);
    }
    drop(map);
    router
}

fn request(model: &str, trace: &Trace) -> ChatRequest {
    let request: ChatRequest =
        serde_json::from_str(&format!(r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#))
            .unwrap();
    request.with_trace(trace.clone())
}

#[tokio::test]
async fn a_picked_child_routes_with_its_own_strategy_and_records_the_path() {
    let direct = Backend::new("direct");
    let low = Backend::new("low");
    let high = Backend::new("high");
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![entry(&direct, 1)], vec![child("kid", 100, None)], StrategyKind::Priority),
        ),
        (
            "kid",
            RoutingTable::with_strategy(vec![entry(&low, 1), entry(&high, 50)], StrategyKind::Priority),
        ),
    ])
    .await;

    for _ in 0..3 {
        let trace = Trace::new();
        router.chat_completions(&request("parent", &trace), None).await.unwrap();
        assert_eq!(trace.route().as_deref(), Some("parent > kid > high"));
    }
    assert_eq!((direct.calls(), low.calls(), high.calls()), (0, 0, 3));
}

#[tokio::test]
async fn a_child_with_every_provider_down_fails_over_to_the_parents_next_member() {
    let direct = Backend::new("direct");
    let a = Backend::new("a");
    let b = Backend::new("b");
    a.failing.store(true, Ordering::SeqCst);
    b.failing.store(true, Ordering::SeqCst);
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![entry(&direct, 1)], vec![child("kid", 100, None)], StrategyKind::Priority),
        ),
        ("kid", RoutingTable::with_strategy(vec![entry(&a, 2), entry(&b, 1)], StrategyKind::Priority)),
    ])
    .await;

    let trace = Trace::new();
    router.chat_completions(&request("parent", &trace), None).await.unwrap();
    assert_eq!(trace.route().as_deref(), Some("parent > direct"));
    assert_eq!((a.calls(), b.calls(), direct.calls()), (1, 1, 1));
}

#[tokio::test]
async fn a_child_whose_providers_are_all_unhealthy_is_skipped_while_a_direct_provider_is_healthy() {
    let direct = Backend::new("direct");
    let down = Backend::new("down");
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![entry(&direct, 1)], vec![child("kid", 100, None)], StrategyKind::Priority),
        ),
        ("kid", RoutingTable::with_strategy(vec![entry(&down, 1)], StrategyKind::Priority)),
    ])
    .await;
    for _ in 0..5 {
        router
            .metrics_store
            .record(crate::metrics::ProviderMetrics {
                provider: "down".into(),
                model: String::new(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
                event: crate::metrics::MetricsEvent::Failure(crate::metrics::FailureDetails {
                    error_type: crate::metrics::ErrorType::Other,
                    error_code: None,
                    error_message: "down".into(),
                    retry_after_ms: None,
                    status_code: None,
                }),
                user: None,
            })
            .await;
    }
    let routed = router.collect_routed("parent", None).await.unwrap();
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0].route, "parent > direct");
}

#[tokio::test]
async fn extra_body_merges_from_the_leaf_entry_up_to_the_outermost_config() {
    let leaf = Backend::new("leaf");
    let router = router_with(vec![
        (
            "top",
            RoutingTable::with_children(
                vec![],
                vec![child_with_body("mid", body(serde_json::json!({"a": "top", "only_top": 1})))],
                StrategyKind::Priority,
            ),
        ),
        (
            "mid",
            RoutingTable::with_children(
                vec![],
                vec![child_with_body("low", body(serde_json::json!({"a": "mid", "b": "mid", "only_mid": 1})))],
                StrategyKind::Priority,
            ),
        ),
        (
            "low",
            RoutingTable::with_strategy(
                vec![entry_with_body(
                    &leaf,
                    body(serde_json::json!({"a": "leaf", "b": "leaf", "c": "leaf", "provider": {"order": ["x"]}})),
                )],
                StrategyKind::Priority,
            ),
        ),
    ])
    .await;
    let routed = router.collect_routed("top", None).await.unwrap();
    assert_eq!(routed.len(), 1);
    assert_eq!(
        serde_json::Value::Object(routed[0].extra_body.clone()),
        serde_json::json!({
            "a": "top",
            "b": "mid",
            "c": "leaf",
            "only_top": 1,
            "only_mid": 1,
            "provider": {"order": ["x"]}
        })
    );
}

#[tokio::test]
async fn a_child_member_model_overrides_the_childs_model_name() {
    let leaf = Backend::new("leaf");
    let mapped = ProviderEntry { provider: leaf.clone(), model_override: Some("pinned".into()), weight: 1, extra_body: Default::default() };
    let plain = Backend::new("plain");
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![], vec![child("renamed", 1, Some("child-model")), child("keeps", 1, None)], StrategyKind::Priority),
        ),
        ("renamed", RoutingTable::with_strategy(vec![entry(&plain, 1)], StrategyKind::Priority)),
        ("keeps", RoutingTable::with_strategy(vec![mapped], StrategyKind::Priority)),
    ])
    .await;
    let routed = router.collect_routed("parent", None).await.unwrap();
    let models: Vec<(&str, &str)> = routed.iter().map(|c| (c.provider.name(), c.model.as_str())).collect();
    assert_eq!(models, vec![("plain", "child-model"), ("leaf", "pinned")]);
}

#[tokio::test]
async fn nesting_deeper_than_the_cap_errors_instead_of_looping() {
    let leaf = Backend::new("leaf");
    let mut tables = vec![];
    for i in 0..8 {
        let name = format!("c{i}");
        let table = if i == 7 {
            RoutingTable::with_strategy(vec![entry(&leaf, 1)], StrategyKind::Priority)
        } else {
            RoutingTable::with_children(vec![], vec![child(&format!("c{}", i + 1), 1, None)], StrategyKind::Priority)
        };
        tables.push((name, table));
    }
    let router = router_with(tables.iter().map(|(n, t)| (n.as_str(), RoutingTable::with_children(t.entries.clone(), t.children.clone(), t.strategy))).collect()).await;
    assert!(matches!(router.collect_routed("c0", None).await, Err(RouterError::NestingTooDeep(_))));

    let looped = router_with(vec![(
        "loop",
        RoutingTable::with_children(vec![], vec![child("loop", 1, None)], StrategyKind::Priority),
    )])
    .await;
    assert!(matches!(looped.collect_routed("loop", None).await, Err(RouterError::NestingTooDeep(_))));
}

#[tokio::test]
async fn candidate_backends_lists_leaves_of_nested_configs() {
    let direct = Backend::new("direct");
    let leaf = Backend::new("leaf");
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![entry(&direct, 1)], vec![child("kid", 1, None)], StrategyKind::Priority),
        ),
        ("kid", RoutingTable::with_strategy(vec![entry(&leaf, 1)], StrategyKind::Priority)),
    ])
    .await;
    let names: Vec<String> = router.candidate_backends("parent").await.iter().map(|(p, _)| p.name().to_string()).collect();
    assert_eq!(names, vec!["direct", "leaf"]);
}

#[tokio::test]
async fn pick_order_walks_nested_configs_in_priority_order() {
    let direct = Backend::new("direct");
    let low = Backend::new("low");
    let high = Backend::new("high");
    let router = router_with(vec![
        (
            "parent",
            RoutingTable::with_children(vec![entry(&direct, 1)], vec![child("kid", 100, None)], StrategyKind::Priority),
        ),
        ("kid", RoutingTable::with_strategy(vec![entry(&low, 1), entry(&high, 50)], StrategyKind::Priority)),
    ])
    .await;
    let (priority, backends) = router.pick_order("parent").await;
    let names: Vec<&str> = backends.iter().map(|(p, _)| p.name()).collect();
    assert!(priority);
    assert_eq!(names, vec!["high", "low", "direct"]);
}
