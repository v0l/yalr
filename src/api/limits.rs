use std::time::Duration;

use crate::providers::ListedLimits;
use crate::state::AppState;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub fn smallest(reported: impl IntoIterator<Item = ListedLimits>) -> ListedLimits {
    let mut out = ListedLimits::default();
    for limits in reported {
        out.context_length = min_some(out.context_length, limits.context_length);
        out.max_output_tokens = min_some(out.max_output_tokens, limits.max_output_tokens);
    }
    out
}

fn min_some(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub async fn routing_limits(state: &AppState, model: &str) -> ListedLimits {
    let backends = state.config.router.candidate_backends(model).await;
    let mut reported = Vec::new();
    for (provider, resolved) in backends {
        let limits = provider_limits(state, &provider, &resolved).await;
        if !limits.is_empty() {
            reported.push(limits);
        }
    }
    smallest(reported)
}

pub async fn provider_limits(
    state: &AppState,
    provider: &std::sync::Arc<dyn crate::providers::Provider>,
    model: &str,
) -> ListedLimits {
    let key = format!("{}/{}", provider.slug(), model);
    if let Some(cached) = state.limits_cache.get(&key).await {
        return cached;
    }
    if !state.metrics_store.is_provider_available(provider.name()).await {
        return ListedLimits::default();
    }
    match tokio::time::timeout(PROBE_TIMEOUT, provider.listed_limits(model)).await {
        Ok(found) => {
            let limits = found.unwrap_or_default();
            state.limits_cache.put(&key, limits).await;
            limits
        }
        Err(_) => ListedLimits::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(c: Option<u32>, o: Option<u32>) -> ListedLimits {
        ListedLimits { context_length: c, max_output_tokens: o }
    }

    #[test]
    fn pool_window_is_the_smallest_reported() {
        let got = smallest([l(Some(1_000_000), Some(64_000)), l(Some(200_000), None), l(Some(262_144), Some(32_000))]);
        assert_eq!((got.context_length, got.max_output_tokens), (Some(200_000), Some(32_000)));
    }

    #[test]
    fn backends_that_report_nothing_are_skipped_not_zero() {
        let got = smallest([l(None, None), l(Some(128_000), None), ListedLimits::default()]);
        assert_eq!((got.context_length, got.max_output_tokens), (Some(128_000), None));
    }

    #[test]
    fn nothing_reported_stays_unknown() {
        assert!(smallest([]).is_empty());
    }
}
