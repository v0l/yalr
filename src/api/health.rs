use crate::api::tenant::{Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::state::AppState;
use axum::{extract::State, Json};
use serde::Serialize;

const RECENT_EVENTS: usize = 50;

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
    /// Most-consumed quota window ("first to stop") for compact card display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<crate::providers::provider_trait::QuotaSnapshot>,
    /// All enforced quota windows for this provider (for the detail modal).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub quotas: Vec<crate::providers::provider_trait::QuotaSnapshot>,
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
    pub transient_retries: u64,
    pub transient_retry_successes: u64,
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

    let quotas = state.metrics_store.get_quota(name).await;

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
        quota: quotas
            .as_deref()
            .and_then(crate::providers::quota::worst_quota),
        quotas: quotas.unwrap_or_default(),
    }
}

struct Visible(Option<std::collections::HashSet<String>>, i64, bool);

impl Visible {
    async fn of(state: &AppState, user: UserExtractor) -> Result<Self, TenantError> {
        let scope = Scope::from(user);
        let names = scope.provider_names(state).await?;
        Ok(Self(names.map(|n| n.into_iter().collect()), scope.user.id, scope.is_admin()))
    }

    fn scrub(&self, event: crate::metrics::ProviderMetrics) -> crate::metrics::ProviderMetrics {
        event.without_other_callers(self.1, self.2)
    }

    fn admits(&self, provider: &str) -> bool {
        self.0.as_ref().is_none_or(|names| names.contains(provider))
    }

    fn is_tenant(&self) -> bool {
        self.0.is_some()
    }
}

fn outcome_counts(events: &[crate::metrics::ProviderMetrics]) -> (u64, u64, u64) {
    let successes = events.iter().filter(|e| matches!(e.event, crate::metrics::MetricsEvent::Success)).count() as u64;
    let failures = events.iter().filter(|e| matches!(e.event, crate::metrics::MetricsEvent::Failure(_))).count() as u64;
    (successes + failures, successes, failures)
}

pub async fn get_metrics(
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<MetricsResponse>, TenantError> {
    let visible = Visible::of(&state, user).await?;
    let providers: Vec<_> = state.config.router.get_providers().await.into_iter().filter(|p| visible.admits(p.name())).collect();
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

    let events: Vec<_> = state
        .metrics_store
        .recent_events(usize::MAX)
        .await
        .into_iter()
        .filter(|e| visible.admits(&e.provider))
        .map(|e| visible.scrub(e))
        .collect();
    let recent_events = events.iter().take(RECENT_EVENTS).map(|e| serde_json::to_value(e).unwrap_or_default()).collect();
    let ((total_requests, total_successes, total_failures), (transient_retries, transient_retry_successes)) = if visible.is_tenant() {
        (outcome_counts(&events), (0, 0))
    } else {
        (state.metrics_store.get_total_requests(), state.metrics_store.emitter().get_transient_retries())
    };

    Ok(Json(MetricsResponse {
        providers: provider_metrics,
        recent_events,
        total_requests,
        total_successes,
        total_failures,
        transient_retries,
        transient_retry_successes,
    }))
}

pub async fn get_health_overview(
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<HealthOverviewResponse>, TenantError> {
    let visible = Visible::of(&state, user).await?;
    let mut health_entries = Vec::new();
    for provider in state.config.router.get_providers().await.iter().filter(|p| visible.admits(p.name())) {
        health_entries.push(build_provider_health_entry(&state, provider.as_ref()).await);
    }
    Ok(Json(HealthOverviewResponse {
        provider_count: health_entries.len(),
        unhealthy_count: health_entries.iter().filter(|e| e.health_state == "unhealthy").count(),
        degraded_count: health_entries.iter().filter(|e| e.health_state == "degraded").count(),
        providers: health_entries,
    }))
}

pub async fn get_metrics_history(
    State(state): State<std::sync::Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<Vec<crate::metrics::MetricsSnapshot>>, TenantError> {
    let visible = Visible::of(&state, user).await?;
    let mut history = state.metrics_store.get_history().await;
    if visible.is_tenant() {
        for snapshot in &mut history {
            snapshot.providers.retain(|p| visible.admits(&p.provider));
            snapshot.provider_health.retain(|p| visible.admits(&p.provider));
        }
    }
    Ok(Json(history))
}
