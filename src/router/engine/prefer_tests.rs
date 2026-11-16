use super::full_name_tests::{add_owned, chat, world, Listed, World};
use super::*;
use crate::db::{Record, RoutingPreferences};
use crate::request_log::Trace;
use crate::router::preference::{Preference, MIN_SPEED_SAMPLES};

const MODEL: &str = "acme/m-1";

async fn pool(w: &World, prices: &[(&str, i64)]) -> Vec<Arc<Listed>> {
    let mut backends = Vec::new();
    for (name, price) in prices {
        let backend = Listed::new(name, &[MODEL]);
        add_owned(w, None, &backend, MODEL, Some(*price)).await;
        backends.push(backend);
    }
    backends
}

async fn warm(w: &World) {
    w.router.collect_routed_preferring(MODEL, None, None, Some(Preference::Fastest)).await.unwrap();
    for _ in 0..200 {
        {
            let board = w.router.speeds.read().await;
            if !board.refreshing && board.refreshes > 0 {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("speed board never refreshed");
}

async fn first(w: &World, viewer: Option<i64>, requested: Option<Preference>) -> String {
    let ranked = w.router.collect_routed_preferring(MODEL, None, viewer, requested).await.unwrap();
    ranked[0].provider.name().to_string()
}

async fn sample(w: &World, provider: &str, n: i64, total_ms: i64, first_ms: i64, tokens: i64) {
    let now = chrono::Utc::now().timestamp_millis();
    for i in 0..n {
        w.db.insert_request_log(Record {
            request_id: format!("{provider}-{i}"),
            started_at_ms: now - 1000,
            model: MODEL.into(),
            provider: Some(provider.into()),
            status: "ok".into(),
            first_event_ms: Some(first_ms),
            total_ms,
            completion_tokens: Some(tokens),
            ..Default::default()
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn cheapest_picks_the_lowest_price_and_balanced_still_rotates() {
    let w = world().await;
    pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000), ("mid", 2_000_000)]).await;
    for _ in 0..3 {
        assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "cheap");
    }
    let mut seen = std::collections::HashSet::new();
    for _ in 0..3 {
        seen.insert(first(&w, None, None).await);
    }
    assert!(seen.len() > 1);
}

#[tokio::test]
async fn equal_prices_fall_back_to_the_balanced_rotation() {
    let w = world().await;
    pool(&w, &[("a", 1_000_000), ("b", 1_000_000)]).await;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        seen.insert(first(&w, None, Some(Preference::Cheapest)).await);
    }
    assert_eq!(seen.len(), 2);
}

#[tokio::test]
async fn fastest_picks_the_highest_tokens_per_second_and_an_unsampled_member_is_the_median() {
    let w = world().await;
    pool(&w, &[("slow", 1), ("quick", 1), ("new", 1)]).await;
    sample(&w, "slow", 6, 10_000, 500, 100).await;
    sample(&w, "quick", 6, 2_000, 500, 100).await;
    warm(&w).await;
    assert_eq!(first(&w, None, Some(Preference::Fastest)).await, "quick");
    let ranked = w.router.collect_routed_preferring(MODEL, None, None, Some(Preference::Fastest)).await.unwrap();
    let names: Vec<&str> = ranked.iter().map(|c| c.provider.name()).collect();
    assert_eq!(names.len(), 3);
    assert_eq!(names[0], "quick");
    assert_ne!(names[2], "new");
}

#[tokio::test]
async fn precedence_is_request_then_model_then_user_then_balanced() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000), ("quick", 2_000_000)]).await;
    sample(&w, "dear", 6, 10_000, 500, 100).await;
    sample(&w, "cheap", 6, 8_000, 500, 100).await;
    sample(&w, "quick", 6, 1_000, 500, 100).await;
    warm(&w).await;
    w.db.replace_routing_preferences(
        alice.id,
        &RoutingPreferences { default: Some("cheapest".into()), models: vec![(MODEL.into(), "fastest".into())] },
    )
    .await
    .unwrap();
    assert_eq!(first(&w, Some(alice.id), Some(Preference::Cheapest)).await, "cheap");
    assert_eq!(first(&w, Some(alice.id), None).await, "quick");
    w.db.replace_routing_preferences(alice.id, &RoutingPreferences { default: Some("cheapest".into()), models: vec![] })
        .await
        .unwrap();
    assert_eq!(first(&w, Some(alice.id), None).await, "cheap");
}

#[tokio::test]
async fn the_nitro_suffix_is_stripped_before_the_pool_lookup() {
    let w = world().await;
    let backends = pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000)]).await;
    sample(&w, "dear", 6, 1_000, 100, 100).await;
    sample(&w, "cheap", 6, 9_000, 100, 100).await;
    warm(&w).await;
    for _ in 0..3 {
        w.router.chat_completions(&chat("acme/m-1:nitro", &Trace::new()), None).await.unwrap();
    }
    assert_eq!((backends[0].calls(), backends[1].calls()), (3, 0));
    for _ in 0..2 {
        w.router.chat_completions(&chat("acme/m-1:floor", &Trace::new()), None).await.unwrap();
    }
    assert_eq!((backends[0].calls(), backends[1].calls()), (3, 2));
}

#[tokio::test]
async fn a_cooling_cheap_provider_is_not_chosen_over_a_healthy_one() {
    let w = world().await;
    pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000)]).await;
    w.metrics
        .record(crate::metrics::ProviderMetrics {
            provider: "cheap".to_string(),
            model: MODEL.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
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
    assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "dear");
}

#[tokio::test]
async fn a_provider_at_max_concurrency_is_skipped_for_the_next() {
    let w = world().await;
    let ids = pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000)]).await;
    assert_eq!(ids.len(), 2);
    let record = w.db.get_provider_by_slug("cheap").await.unwrap().unwrap();
    w.db.set_provider_max_concurrency(record.id, Some(1)).await.unwrap();
    w.router.refresh_full_name_index().await;
    assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "cheap");
    w.metrics.increment_in_flight("cheap").await;
    assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "dear");
}

#[tokio::test]
async fn the_sticky_pin_only_wins_inside_the_preference_band() {
    let w = world().await;
    pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000)]).await;
    let key = SessionKey::new(None, MODEL, Some("s")).unwrap();
    w.router.sticky.pin(&key, "dear");
    let ranked = w.router.collect_routed_preferring(MODEL, Some(&key), None, Some(Preference::Cheapest)).await.unwrap();
    assert_eq!(ranked[0].provider.name(), "cheap");
    let ranked = w.router.collect_routed_preferring(MODEL, Some(&key), None, None).await.unwrap();
    assert_eq!(ranked[0].provider.name(), "dear");
}

#[tokio::test]
async fn admin_routing_configs_ignore_a_user_preference() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let a = Listed::new("a", &["x"]);
    let b = Listed::new("b", &["x"]);
    w.router.add_provider(a.clone()).await;
    w.router.add_provider(b.clone()).await;
    w.router.register_route("code", vec![a.clone(), b.clone()]).await;
    w.db.replace_routing_preferences(alice.id, &RoutingPreferences { default: Some("cheapest".into()), models: vec![] })
        .await
        .unwrap();
    let mut seen = std::collections::HashSet::new();
    for _ in 0..4 {
        let ranked = w.router.collect_routed_preferring("code", None, Some(alice.id), Some(Preference::Cheapest)).await.unwrap();
        seen.insert(ranked[0].provider.name().to_string());
    }
    assert_eq!(seen.len(), 2);
}

#[tokio::test]
async fn a_fastest_request_on_a_stale_board_does_not_wait_for_the_refresh() {
    let w = world().await;
    pool(&w, &[("a", 1_000_000), ("b", 1_000_000)]).await;
    w.router.speeds.write().await.delay = Duration::from_secs(5);
    let started = Instant::now();
    let ranked = w.router.collect_routed_preferring(MODEL, None, None, Some(Preference::Fastest)).await.unwrap();
    assert_eq!(ranked.len(), 2);
    assert!(started.elapsed() < Duration::from_secs(2), "routing waited {:?}", started.elapsed());
    assert!(w.router.speeds.read().await.refreshing);
}

#[tokio::test]
async fn concurrent_stale_requests_trigger_one_refresh() {
    let w = world().await;
    pool(&w, &[("a", 1_000_000), ("b", 1_000_000)]).await;
    w.router.speeds.write().await.delay = Duration::from_millis(300);
    let calls = (0..8).map(|_| w.router.collect_routed_preferring(MODEL, None, None, Some(Preference::Fastest)));
    for ranked in futures::future::join_all(calls).await {
        ranked.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(w.router.speeds.read().await.refreshes, 1);
}

#[tokio::test]
async fn cheapest_prices_are_cached_between_requests() {
    let w = world().await;
    pool(&w, &[("dear", 3_000_000), ("cheap", 1_000_000)]).await;
    assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "cheap");
    w.db.execute_raw("DELETE FROM model_pricing").await.unwrap();
    assert_eq!(first(&w, None, Some(Preference::Cheapest)).await, "cheap");
}

#[tokio::test]
async fn the_speed_rollup_aggregates_in_sql_and_drops_thin_samples() {
    let w = world().await;
    pool(&w, &[("slow", 1), ("quick", 1)]).await;
    sample(&w, "slow", 6, 10_000, 500, 100).await;
    sample(&w, "quick", 2, 2_000, 500, 100).await;
    let rows = w.db.speed_rollup(0, MIN_SPEED_SAMPLES).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].provider, "slow");
}
