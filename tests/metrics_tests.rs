use yalr::metrics::{MetricsEmitter, MetricsEvent, MetricsStore, ProviderMetrics, FailureDetails};

fn create_test_metrics() -> (MetricsEmitter, MetricsStore) {
    let (emitter, _) = MetricsEmitter::new(1000);
    let store = MetricsStore::new(emitter.clone(), 100);
    (emitter, store)
}

fn create_test_event(provider: &str, model: &str, event: MetricsEvent) -> ProviderMetrics {
    ProviderMetrics {
        provider: provider.to_string(),
        model: model.to_string(),
        timestamp_ms: 0,
        event,
    }
}

#[tokio::test]
async fn test_metrics_emitter_success() {
    let (_emitter, store) = create_test_metrics();
    
    let event = create_test_event("provider1", "model1", MetricsEvent::Success);
    store.record(event).await;
    
    let events = store.get_events_for("provider1", Some("model1")).await;
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].event, MetricsEvent::Success));
}

#[tokio::test]
async fn test_metrics_emitter_failure() {
    let (_emitter, store) = create_test_metrics();
    
    let event = create_test_event("provider1", "model1", MetricsEvent::Failure(FailureDetails {
        error_type: yalr::metrics::ErrorType::ServerError,
        error_code: None,
        error_message: "test error".to_string(),
        retry_after_ms: None,
        status_code: None,
    }));
    store.record(event).await;
    
    let events = store.get_events_for("provider1", Some("model1")).await;
    assert_eq!(events.len(), 1);
    
    if let MetricsEvent::Failure(error) = &events[0].event {
        assert_eq!(error.error_message, "test error");
    } else {
        panic!("Expected Failure event");
    }
}

#[tokio::test]
async fn test_metrics_ttft() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::TTFT(50))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::TTFT(60))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::TTFT(70))).await;
    
    let p90 = store.p90_ttft("provider1", Some("model1")).await;
    assert!(p90.is_some());
    assert!(p90.unwrap() >= 60);
}

#[tokio::test]
async fn test_metrics_output_tokens_per_second() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::OutputTokensPerSecond(100.0))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::OutputTokensPerSecond(200.0))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::OutputTokensPerSecond(300.0))).await;
    
    let p90 = store.p90_output_tokens_per_second("provider1", Some("model1")).await;
    assert!(p90.is_some());
    assert!(p90.unwrap() >= 200.0);
}

#[tokio::test]
async fn test_metrics_avg_latency() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::TotalLatency(100))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::TotalLatency(200))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::TotalLatency(300))).await;
    
    let avg = store.avg_latency("provider1", Some("model1")).await;
    assert!(avg.is_some());
    assert_eq!(avg.unwrap(), 200.0);
}

#[tokio::test]
async fn test_metrics_success_rate() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::Failure(FailureDetails {
        error_type: yalr::metrics::ErrorType::ServerError,
        error_code: None,
        error_message: "error".to_string(),
        retry_after_ms: None,
        status_code: None,
    }))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::Success)).await;
    
    let rate = store.success_rate("provider1", Some("model1")).await;
    assert!(rate.is_some());
    assert_eq!(rate.unwrap(), 0.75);
}

#[tokio::test]
async fn test_metrics_provider_aggregation() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model2", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::Failure(FailureDetails {
        error_type: yalr::metrics::ErrorType::ServerError,
        error_code: None,
        error_message: "error".to_string(),
        retry_after_ms: None,
        status_code: None,
    }))).await;
    
    let summary = store.get_provider_summary("provider1").await;
    assert_eq!(summary.provider, "provider1");
    assert!(summary.success_rate.is_some());
}

#[tokio::test]
async fn test_metrics_model_specific() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::TTFT(50))).await;
    store.record(create_test_event("provider1", "model2", MetricsEvent::TTFT(100))).await;
    
    let model1_ttft = store.p90_ttft("provider1", Some("model1")).await;
    let model2_ttft = store.p90_ttft("provider1", Some("model2")).await;
    
    assert!(model1_ttft.is_some());
    assert!(model2_ttft.is_some());
    assert_eq!(model1_ttft.unwrap(), 50);
    assert_eq!(model2_ttft.unwrap(), 100);
}

#[tokio::test]
async fn test_metrics_empty_provider() {
    let (_emitter, store) = create_test_metrics();
    
    let summary = store.get_provider_summary("nonexistent").await;
    assert_eq!(summary.provider, "nonexistent");
    assert!(summary.p90_ttft.is_none());
    assert!(summary.avg_latency.is_none());
}

#[tokio::test]
async fn test_metrics_event_count_limit() {
    let (emitter, _) = MetricsEmitter::new(1000);
    let store = MetricsStore::new(emitter, 10);
    
    for _ in 0..20 {
        let event = create_test_event("provider1", "model1", MetricsEvent::Success);
        store.record(event).await;
    }
    
    let events = store.get_events_for("provider1", Some("model1")).await;
    assert_eq!(events.len(), 10);
}

#[tokio::test]
async fn test_metrics_recent_events() {
    let (_emitter, store) = create_test_metrics();
    
    for i in 0..5 {
        store.record(create_test_event("provider1", "model1", MetricsEvent::TTFT(i as u32))).await;
    }
    
    let recent = store.recent_events(3).await;
    assert_eq!(recent.len(), 3);
}

#[tokio::test]
async fn test_metrics_token_counts() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::InputTokens(100))).await;
    store.record(create_test_event("provider1", "model1", MetricsEvent::OutputTokens(200))).await;
    
    let events = store.get_events_for("provider1", Some("model1")).await;
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn test_metrics_no_model_filter() {
    let (_emitter, store) = create_test_metrics();
    
    store.record(create_test_event("provider1", "model1", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model2", MetricsEvent::Success)).await;
    store.record(create_test_event("provider1", "model3", MetricsEvent::Success)).await;
    
    let events = store.get_events_for("provider1", None).await;
    assert_eq!(events.len(), 3);
}
