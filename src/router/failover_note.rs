use crate::metrics::{HealthState, MetricsStore};
use crate::providers::attribution::FailureAttribution;
use crate::providers::{ChatRequest, ProviderError};
use crate::request_log::fingerprint::fingerprint;
use crate::request_log::FailoverNote;
use crate::router::sticky::{expected_cached_prefix, LastServed};

pub(crate) struct FailedAttempt {
    pub provider: String,
    pub attribution: FailureAttribution,
    pub error: String,
}

impl FailedAttempt {
    pub fn new(provider: &str, error: &ProviderError) -> Self {
        Self { provider: provider.to_string(), attribution: error.attribution(), error: error.to_string() }
    }
}

pub(crate) fn turn_hashes(request: &ChatRequest) -> Vec<String> {
    serde_json::to_value(&**request).map(|body| fingerprint(&body).message_hashes).unwrap_or_default()
}

pub(crate) fn response_hashes(request: &async_openai::types::responses::CreateResponse) -> Vec<String> {
    let Ok(mut body) = serde_json::to_value(request) else {
        return Vec::new();
    };
    let items = match body.get_mut("input").map(serde_json::Value::take) {
        Some(serde_json::Value::Array(items)) => items,
        Some(other) if !other.is_null() => vec![other],
        _ => Vec::new(),
    };
    fingerprint(&serde_json::json!({ "messages": items })).message_hashes
}

#[cfg(test)]
pub(crate) fn response_hashes_of(items: &[String]) -> Vec<String> {
    let items: Vec<serde_json::Value> = items.iter().map(|i| serde_json::Value::String(i.clone())).collect();
    fingerprint(&serde_json::json!({ "messages": items })).message_hashes
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
        Some(attempt) if attempt.attribution == FailureAttribution::Provider => attempt.error.clone(),
        Some(_) => return None,
        None if is_unhealthy(metrics, &previous.provider).await => "provider unavailable".to_string(),
        None => return None,
    };
    Some(FailoverNote { provider: previous.provider.clone(), expected_cached, reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::sticky::Turn;

    fn previous() -> (LastServed, Vec<String>) {
        let hashes = vec!["h1".to_string()];
        (LastServed { provider: "a".into(), turn: Turn { hashes: hashes.clone(), prompt_tokens: Some(10_000) } }, hashes)
    }

    fn failed(attribution: FailureAttribution) -> Vec<FailedAttempt> {
        vec![FailedAttempt { provider: "a".into(), attribution, error: "boom".into() }]
    }

    async fn note_for(attribution: FailureAttribution) -> Option<FailoverNote> {
        let (previous, hashes) = previous();
        failover_note(&MetricsStore::new(10), Some(&previous), "b", &hashes, &failed(attribution)).await
    }

    #[tokio::test]
    async fn a_provider_fault_is_noted() {
        assert_eq!(note_for(FailureAttribution::Provider).await.map(|n| n.expected_cached), Some(10_000));
    }

    #[tokio::test]
    async fn a_request_fault_is_not_noted() {
        assert!(note_for(FailureAttribution::Request).await.is_none());
    }

    #[tokio::test]
    async fn an_unknown_attribution_charges_no_penalty() {
        assert!(note_for(FailureAttribution::Unknown).await.is_none());
    }
}
