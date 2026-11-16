use crate::metrics::{HealthState, MetricsStore};
use crate::providers::{ChatRequest, ProviderError};
use crate::request_log::fingerprint::fingerprint;
use crate::request_log::FailoverNote;
use crate::router::sticky::{expected_cached_prefix, LastServed};

pub(crate) struct FailedAttempt {
    pub provider: String,
    pub provider_fault: bool,
    pub error: String,
}

impl FailedAttempt {
    pub fn new(provider: &str, error: &ProviderError) -> Self {
        Self { provider: provider.to_string(), provider_fault: !error.is_client_error(), error: error.to_string() }
    }
}

pub(crate) fn turn_hashes(request: &ChatRequest) -> Vec<String> {
    serde_json::to_value(&**request).map(|body| fingerprint(&body).message_hashes).unwrap_or_default()
}

async fn is_unhealthy(metrics: &MetricsStore, provider: &str) -> bool {
    let snapshot = metrics.routing_snapshot(&[provider]).await;
    snapshot
        .get(provider)
        .is_some_and(|h| !h.available || h.health != HealthState::Healthy || !h.retry_in.is_zero())
}

pub(crate) async fn failover_note(
    metrics: &MetricsStore,
    previous: Option<&LastServed>,
    served: &str,
    hashes: &[String],
    failed: &[FailedAttempt],
) -> Option<FailoverNote> {
    let previous = previous.filter(|p| p.provider != served)?;
    let expected_cached = expected_cached_prefix(&previous.turn, hashes);
    if expected_cached == 0 {
        return None;
    }
    let reason = match failed.iter().find(|f| f.provider == previous.provider) {
        Some(attempt) if attempt.provider_fault => attempt.error.clone(),
        Some(_) => return None,
        None if is_unhealthy(metrics, &previous.provider).await => "provider unavailable".to_string(),
        None => return None,
    };
    Some(FailoverNote { provider: previous.provider.clone(), expected_cached, reason })
}
