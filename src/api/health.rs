use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub timestamp: u64,
}

#[derive(Serialize)]
pub struct ProviderMetrics {
    pub provider: String,
    pub p90_tokens_per_second: Option<f32>,
    pub p90_input_tokens_per_second: Option<f32>,
    pub p90_ttft_ms: Option<u32>,
    pub avg_latency_ms: Option<f32>,
    pub success_rate: Option<f32>,
    pub health_state: Option<String>,
    pub consecutive_failures: Option<u32>,
    pub in_flight: Option<u32>,
    pub max_concurrency: Option<u32>,
    pub backoff_ms: Option<u64>,
    pub load_score: Option<f32>,
    pub available: Option<bool>,
    /// Full health entry with balance tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ProviderHealthEntry>,
}

#[derive(Serialize)]
pub struct ProviderHealthEntry {
    pub provider: String,
    pub health_state: String,
    pub consecutive_failures: u32,
    pub in_flight: u32,
    pub max_concurrency: Option<u32>,
    pub load_score: Option<f32>,
    pub backoff_ms: u64,
    pub available: bool,
    pub last_failure_ago_ms: Option<u64>,
    pub rate_limited: bool,
    /// Current balance for this provider, if it supports balance tracking.
    /// Serialized as `{"currency": "msats"|"sats"|"usd_micro", "amount": N}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<crate::providers::provider_trait::CurrencyAmount>,
    /// Current usage quota for this provider, if it supports quota tracking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<crate::providers::provider_trait::QuotaSnapshot>,
}

#[derive(Serialize)]
pub struct HealthOverviewResponse {
    pub providers: Vec<ProviderHealthEntry>,
    pub provider_count: usize,
    pub unhealthy_count: usize,
    pub degraded_count: usize,
}

#[derive(Serialize)]
pub struct MetricsResponse {
    pub providers: Vec<ProviderMetrics>,
    pub recent_events: Vec<serde_json::Value>,
    pub total_requests: u64,
    pub total_successes: u64,
    pub total_failures: u64,
}

pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    })
}

/// Build a ProviderHealthEntry for a live provider by looking up metrics,
/// health state, and balance from the AppState.
pub async fn build_provider_health_entry(
    state: &std::sync::Arc<AppState>,
    provider: &dyn crate::providers::Provider,
) -> ProviderHealthEntry {
    use std::time::Instant;
    let name = provider.name();
    let health = state.metrics_store.get_provider_health(name).await;
    let failures = state.metrics_store.get_recent_failures(name).await;
    let backoff = state.metrics_store.get_provider_backoff(name).await;
    let load_score = state.metrics_store.get_provider_load_score(name).await;
    let available = state.metrics_store.is_provider_available(name).await;
    let (in_flight, max_concurrency): (u32, Option<u32>) =
        state.metrics_store.get_provider_load(name).await.unwrap_or((0, None));

    let now = Instant::now();
    let last_failure_ago_ms = {
        state.metrics_store.events.lock().ok().and_then(|ev| {
            ev.iter().rev()
                .find(|e| e.provider == name && matches!(e.event, crate::metrics::MetricsEvent::Failure(_)))
                .map(|_| now.elapsed().as_millis() as u64)
        })
    };
    let rate_limited = {
        state.metrics_store.events.lock().ok().is_some_and(|ev| {
            ev.iter().rev().any(|e| {
                e.provider == name && matches!(&e.event, crate::metrics::MetricsEvent::Failure(d) if d.error_type == crate::metrics::ErrorType::RateLimit)
            })
        })
    };

    ProviderHealthEntry {
        provider: name.to_string(),
        health_state: format!("{:?}", health).to_lowercase(),
        consecutive_failures: failures,
        in_flight,
        max_concurrency,
        load_score,
        backoff_ms: backoff.as_millis() as u64,
        available,
        last_failure_ago_ms,
        rate_limited,
        balance: state.metrics_store.get_balance(name).await,
        quota: state.metrics_store.get_quota(name).await,
    }
}

pub async fn get_metrics(State(state): State<std::sync::Arc<AppState>>) -> Json<MetricsResponse> {
    let providers = state.config.router.get_providers().await;
    let mut provider_metrics = Vec::new();

    for provider in &providers {
        let provider_name = provider.name();
        let summary = state.metrics_store.get_provider_summary(provider_name).await;
        let health = state.metrics_store.get_provider_health(provider_name).await;
        let failures = state.metrics_store.get_recent_failures(provider_name).await;
        let backoff = state.metrics_store.get_provider_backoff(provider_name).await;
        let load_score = state.metrics_store.get_provider_load_score(provider_name).await;
        let available = state.metrics_store.is_provider_available(provider_name).await;
        let (in_flight, max_concurrency): (u32, Option<u32>) = state.metrics_store.get_provider_load(provider_name).await.unwrap_or((0, None));

        provider_metrics.push(ProviderMetrics {
            provider: summary.provider,
            p90_tokens_per_second: summary.p90_output_tokens_per_second,
            p90_input_tokens_per_second: summary.p90_input_tokens_per_second,
            p90_ttft_ms: summary.p90_ttft,
            avg_latency_ms: summary.avg_latency,
            success_rate: summary.success_rate,
            health_state: Some(format!("{:?}", health).to_lowercase()),
            consecutive_failures: Some(failures),
            in_flight: Some(in_flight),
            max_concurrency,
            backoff_ms: Some(backoff.as_millis() as u64),
            load_score,
            available: Some(available),
            health: Some(build_provider_health_entry(&state, provider.as_ref()).await),
        });
    }

    let recent_events: Vec<serde_json::Value> = state
        .metrics_store
        .recent_events(50)
        .await
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or_default())
        .collect();

    let (total_requests, total_successes, total_failures) = state.metrics_store.get_total_requests();

    Json(MetricsResponse {
        providers: provider_metrics,
        recent_events,
        total_requests,
        total_successes,
        total_failures,
    })
}

pub async fn get_health_overview(State(state): State<std::sync::Arc<AppState>>) -> Json<HealthOverviewResponse> {
    let providers = state.config.router.get_providers().await;
    let mut health_entries = Vec::new();
    let mut unhealthy = 0;
    let mut degraded = 0;

    for provider in &providers {
        let entry = build_provider_health_entry(&state, provider.as_ref()).await;
        if entry.health_state == "unhealthy" { unhealthy += 1; }
        else if entry.health_state == "degraded" { degraded += 1; }
        health_entries.push(entry);
    }

    Json(HealthOverviewResponse {
        providers: health_entries,
        provider_count: providers.len(),
        unhealthy_count: unhealthy,
        degraded_count: degraded,
    })
}

pub async fn get_metrics_history(State(state): State<std::sync::Arc<AppState>>) -> Json<serde_json::Value> {
    let history = state.metrics_store.get_history().await;
    Json(serde_json::to_value(history).unwrap_or_default())
}
