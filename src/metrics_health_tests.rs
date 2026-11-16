use crate::metrics::{HealthState, MetricsStore, MetricsUser};
use crate::ProviderError;

fn caller(id: i64) -> Option<MetricsUser> {
    Some(MetricsUser { id: Some(id), name: None, api_key_id: None, api_key_name: None })
}

fn status(code: u16, message: &str) -> ProviderError {
    ProviderError::ServerError { message: message.into(), status_code: Some(code) }
}

fn refused() -> ProviderError {
    ProviderError::Other("error sending request: tcp connect error: Connection refused (os error 111)".to_string().into())
}

const CONTEXT_OVERFLOW: &str = "This model's maximum context length is 8192 tokens. However, you requested 91234 tokens";

#[tokio::test]
async fn ten_oversized_prompt_500s_from_one_user_do_not_degrade_a_provider() {
    let store = MetricsStore::new(1000);
    for _ in 0..10 {
        store.emitter().emit_provider_error("p", "m", &status(500, CONTEXT_OVERFLOW), caller(1));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Healthy);
    assert_eq!(store.get_recent_failures("p").await, 0);
}

#[tokio::test]
async fn a_busy_provider_at_half_a_percent_errors_stays_healthy() {
    let store = MetricsStore::new(5000);
    for i in 0..995 {
        store.emitter().emit_success("p", "m", caller(i % 50));
    }
    for i in 0..5 {
        store.emitter().emit_provider_error("p", "m", &status(503, "upstream down"), caller(i));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Healthy);
}

#[tokio::test]
async fn a_quiet_provider_failing_three_of_three_from_two_users_degrades() {
    let store = MetricsStore::new(1000);
    for user in [1, 2, 1] {
        store.emitter().emit_provider_error("p", "m", &status(503, "upstream down"), caller(user));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Degraded);
}

#[tokio::test]
async fn three_failures_from_one_user_do_not_degrade_a_quiet_provider() {
    let store = MetricsStore::new(1000);
    for _ in 0..3 {
        store.emitter().emit_provider_error("p", "m", &status(503, "upstream down"), caller(1));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Healthy);
}

#[tokio::test]
async fn connection_refused_three_times_makes_a_provider_unhealthy() {
    let store = MetricsStore::new(1000);
    for _ in 0..3 {
        store.emitter().emit_provider_error("p", "m", &refused(), caller(1));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Unhealthy);
}

#[tokio::test]
async fn half_the_attempts_failing_on_a_busy_provider_makes_it_unhealthy() {
    let store = MetricsStore::new(1000);
    for i in 0..10 {
        store.emitter().emit_success("p", "m", caller(i));
        store.emitter().emit_provider_error("p", "m", &status(503, "upstream down"), caller(10 + i));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Unhealthy);
}

#[tokio::test]
async fn an_unknown_fault_counts_at_half_weight() {
    let store = MetricsStore::new(1000);
    for i in 0..4 {
        store.emitter().emit_success("p", "m", caller(i));
    }
    for i in 0..6 {
        store.emitter().emit_provider_error("p", "m", &status(500, "mystery"), caller(10 + i));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Degraded);
}

#[tokio::test]
async fn a_degraded_provider_recovers_after_two_probe_passes() {
    let store = MetricsStore::new(1000);
    for user in [1, 2, 3] {
        store.emitter().emit_provider_error("p", "m", &status(503, "upstream down"), caller(user));
    }
    assert_eq!(store.get_provider_health("p").await, HealthState::Degraded);
    store.emitter().emit_probe("p", true);
    assert_eq!(store.get_provider_health("p").await, HealthState::Degraded);
    store.emitter().emit_probe("p", true);
    assert_eq!(store.get_provider_health("p").await, HealthState::Healthy);
}
