use super::*;
use crate::db::{NewProvider, ProviderType, UpdateProvider};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HANG: Duration = Duration::from_secs(30);

fn listing(ids: &[&str]) -> serde_json::Value {
    let data: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| serde_json::json!({"id": id, "object": "model", "created": 0, "owned_by": "x"}))
        .collect();
    serde_json::json!({"object": "list", "data": data})
}

async fn backend(ids: &[&str], delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(listing(ids)).set_delay(delay))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/health")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    server
}

async fn model_calls(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/v1/models").count()
}

async fn add(db: &Database, slug: &str, server: &MockServer) -> i64 {
    let url = format!("{}/v1", server.uri());
    db.create_provider(NewProvider {
        name: slug,
        slug,
        base_url: &url,
        api_key: Some("k"),
        provider_type: Some(ProviderType::LlamaCpp),
    })
    .await
    .unwrap()
    .id
}

async fn edit(db: &Database, id: i64, base_url: Option<&str>, api_key: Option<&str>) {
    db.update_provider(
        id,
        UpdateProvider { name: None, slug: None, base_url, api_key: api_key.map(Some), provider_type: None },
    )
    .await
    .unwrap();
}

async fn setup(timeout: Duration) -> (Router, Arc<Database>) {
    let db = Arc::new(Database::test().await);
    let router = Router::new(MetricsStore::new(100), db.clone()).with_list_timeout(timeout);
    (router, db)
}

async fn timed_reload(router: &Router) -> Duration {
    let started = Instant::now();
    router.reload_config().await.unwrap();
    started.elapsed()
}

#[tokio::test]
async fn reload_returns_before_a_hung_provider_answers_its_model_list() {
    let (router, db) = setup(full_name::LIST_TIMEOUT).await;
    let hung = backend(&["acme/slow"], HANG).await;
    add(&db, "hung", &hung).await;
    let took = timed_reload(&router).await;
    assert!(took < Duration::from_millis(500), "reload took {took:?}");
    assert_eq!(router.get_providers().await.len(), 1);
}

#[tokio::test]
async fn the_old_index_answers_while_the_new_one_is_listed_in_the_background() {
    let (router, db) = setup(full_name::LIST_TIMEOUT).await;
    let fast = backend(&["acme/a"], Duration::ZERO).await;
    add(&db, "fast", &fast).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string()]);

    let slow = backend(&["acme/b"], Duration::from_millis(1500)).await;
    add(&db, "slow", &slow).await;
    let took = timed_reload(&router).await;
    assert!(took < Duration::from_millis(500), "reload took {took:?}");
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string()]);

    router.settle_background().await;
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string(), "acme/b".to_string()]);
}

#[tokio::test]
async fn an_unchanged_provider_keeps_its_models_cache_across_a_reload() {
    let (router, db) = setup(full_name::LIST_TIMEOUT).await;
    let kept = backend(&["acme/a"], Duration::ZERO).await;
    add(&db, "kept", &kept).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(model_calls(&kept).await, 1);

    let other = backend(&["acme/b"], Duration::ZERO).await;
    add(&db, "other", &other).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(model_calls(&kept).await, 1);
    assert_eq!(model_calls(&other).await, 1);
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string(), "acme/b".to_string()]);
}

#[tokio::test]
async fn a_provider_that_times_out_keeps_its_previous_listing() {
    let (router, db) = setup(Duration::from_millis(300)).await;
    let first = backend(&["acme/a"], Duration::ZERO).await;
    let id = add(&db, "flaky", &first).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string()]);

    let hung = backend(&["acme/a"], HANG).await;
    edit(&db, id, Some(&format!("{}/v1", hung.uri())), None).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(model_calls(&hung).await, 1);
    assert_eq!(router.full_name_models().await, vec!["acme/a".to_string()]);
}

#[tokio::test]
async fn back_to_back_reloads_run_one_refresh_and_the_latest_config_wins() {
    let (router, db) = setup(Duration::from_secs(2)).await;
    let hung = backend(&["acme/hung"], HANG).await;
    add(&db, "hung", &hung).await;
    let old = backend(&["acme/old"], Duration::from_millis(200)).await;
    let id = add(&db, "moving", &old).await;
    let took_first = timed_reload(&router).await;

    let new = backend(&["acme/new"], Duration::ZERO).await;
    edit(&db, id, Some(&format!("{}/v1", new.uri())), None).await;
    let took_second = timed_reload(&router).await;
    assert!(took_first + took_second < Duration::from_millis(500), "reloads took {took_first:?} and {took_second:?}");
    assert_eq!(router.index_refreshes_running(), 1);

    router.settle_background().await;
    assert_eq!(router.index_refreshes_running(), 0);
    assert_eq!(router.full_name_models().await, vec!["acme/new".to_string()]);
}

#[tokio::test]
async fn reloading_one_provider_keeps_the_running_health_loop() {
    let (router, db) = setup(full_name::LIST_TIMEOUT).await;
    let a = backend(&["acme/a"], Duration::ZERO).await;
    let b = backend(&["acme/b"], Duration::ZERO).await;
    let id = add(&db, "a", &a).await;
    add(&db, "b", &b).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(router.health_loops_started(), 1);

    edit(&db, id, None, Some("k2")).await;
    router.reload_config().await.unwrap();
    router.settle_background().await;
    assert_eq!(router.health_loops_started(), 1);
    assert_eq!(model_calls(&b).await, 1);
    assert_eq!(model_calls(&a).await, 2);
}

#[tokio::test]
async fn a_pending_user_provider_is_never_polled_in_the_background() {
    let db = Arc::new(Database::test().await);
    let router = Router::new(MetricsStore::new(100), db.clone())
        .with_list_timeout(full_name::LIST_TIMEOUT)
        .with_egress(crate::providers::egress::Egress::loopback_for_tests());
    let alice = db.add_test_user("alice", false).await;
    db.create_routing_config(crate::db::NewRoutingConfig {
        name: "fast".into(),
        strategy: "round_robin".into(),
        health_check_enabled: true,
        health_check_interval_seconds: 1,
        health_check_timeout_seconds: 1,
    })
    .await
    .unwrap();
    let catcher = backend(&["acme/a"], Duration::ZERO).await;
    let platform = backend(&["acme/b"], Duration::ZERO).await;
    let url = format!("http://callback-catcher.invalid:{}/v1", catcher.address().port());
    let record = db
        .create_owned_provider(
            alice.id,
            NewProvider { name: "alice/probe", slug: "u1-probe", base_url: &url, api_key: None, provider_type: Some(ProviderType::OpenAi) },
        )
        .await
        .unwrap();
    assert_eq!(record.status, crate::db::PENDING);
    add(&db, "plat", &platform).await;

    router.reload_config().await.unwrap();
    router.settle_background().await;
    router.refresh_full_name_index().await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert!(platform.received_requests().await.unwrap().len() >= 2);
    assert_eq!(catcher.received_requests().await.unwrap().len(), 0);

    db.set_provider_status(record.id, crate::db::APPROVED, None, false).await.unwrap();
    router.reload_config().await.unwrap();
    router.settle_background().await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(catcher.received_requests().await.unwrap().len() >= 2);
}
