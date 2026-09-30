use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::MetricsUser;
use crate::providers::Provider;
use crate::router::engine::{InFlightGuard, Router, RouterError};
use crate::ProviderError;

/// Outcome of one provider attempt, before metrics are recorded.
type Attempt<T> = Result<T, ProviderError>;

/// What one pass over a candidate list produced. `attempts` counts real
/// attempts, so zero means every candidate lacked support entirely.
struct AttemptRun<T> {
    result: Option<Result<T, RouterError>>,
    attempts: u32,
    last_error: Option<RouterError>,
}

impl Router {
    /// Try each candidate backend for `model` in routing order, recording
    /// metrics per attempt and failing over on transient errors.
    ///
    /// `ProviderError::Unsupported` means the backend has no endpoint for this
    /// kind of request at all. A mixed pool is the normal configuration, so that is
    /// neither a failure event nor a used-up retry: the candidate is skipped
    /// silently and its health is left alone.
    pub(crate) async fn capability_failover<T, F, Fut>(
        &self,
        model: &str,
        op: &str,
        user: Option<MetricsUser>,
        call: F,
    ) -> Result<T, RouterError>
    where
        F: Fn(Arc<dyn Provider>, String) -> Fut,
        Fut: std::future::Future<Output = Attempt<T>>,
    {
        let candidates = self.collect_candidates(model).await;
        if candidates.is_empty() {
            return Err(RouterError::NoAvailableProvider);
        }

        let tried: Vec<String> = candidates.iter().map(|(p, _)| p.name().to_string()).collect();
        let outcome = self.try_candidates(candidates, model, op, &user, &call).await;
        if let Some(result) = outcome.result {
            return result;
        }

        // Health filtering can leave only chat-only backends in the candidate
        // list, hiding a capable provider that is merely degraded. Capability
        // beats health here: if nothing in the healthy set could serve the
        // request at all, try the configured backends that were filtered out.
        if outcome.attempts == 0 {
            let fallback: Vec<_> = self
                .candidate_backends(model)
                .await
                .into_iter()
                .filter(|(p, _)| !tried.iter().any(|name| name == p.name()))
                .collect();
            if !fallback.is_empty() {
                tracing::warn!(
                    model,
                    op,
                    count = fallback.len(),
                    "No healthy provider supports this request, falling back to filtered providers"
                );
                if let Some(result) = self
                    .try_candidates(fallback, model, op, &user, &call)
                    .await
                    .result
                {
                    return result;
                }
            }
        }

        Err(outcome.last_error.unwrap_or(RouterError::NoAvailableProvider))
    }

    async fn try_candidates<T, F, Fut>(
        &self,
        candidates: Vec<(Arc<dyn Provider>, String)>,
        model: &str,
        op: &str,
        user: &Option<MetricsUser>,
        call: &F,
    ) -> AttemptRun<T>
    where
        F: Fn(Arc<dyn Provider>, String) -> Fut,
        Fut: std::future::Future<Output = Attempt<T>>,
    {
        let start = Instant::now();
        let mut last_error: Option<RouterError> = None;
        let mut attempt: u32 = 0;

        for (provider, resolved_model) in candidates {
            if attempt >= self.max_retries {
                break;
            }

            let provider_name = provider.name().to_string();
            let in_flight = self.metrics_store.increment_in_flight(&provider_name).await;
            let mut guard = InFlightGuard::new(self.metrics_store.clone(), provider_name.clone());
            self.metrics_store
                .emitter()
                .emit_provider_load(&provider_name, in_flight, None, user.clone());

            let result = call(provider.clone(), resolved_model).await;
            guard.decrement();

            match result {
                Ok(response) => {
                    let latency_ms = start.elapsed().as_millis() as u32;
                    self.metrics_store.emitter().emit_total_latency(
                        &provider_name,
                        model,
                        latency_ms,
                        user.clone(),
                    );
                    self.metrics_store
                        .emitter()
                        .emit_success(&provider_name, model, user.clone());
                    tracing::info!(
                        provider = provider_name,
                        model,
                        op,
                        latency_ms,
                        "Request completed successfully"
                    );
                    return AttemptRun {
                        result: Some(Ok(response)),
                        attempts: attempt,
                        last_error: None,
                    };
                }
                Err(ProviderError::Unsupported(message)) => {
                    tracing::debug!(
                        provider = %provider_name,
                        op,
                        %message,
                        "Provider cannot serve this request type, skipping"
                    );
                    if last_error.is_none() {
                        last_error = Some(RouterError::ProviderError(ProviderError::Unsupported(
                            message,
                        )));
                    }
                }
                Err(e) => {
                    attempt += 1;
                    self.metrics_store.emitter().emit_provider_error(&provider_name, model, &e, user.clone());

                    last_error = Some(RouterError::ProviderError(e.clone()));

                    if !e.is_transient() {
                        tracing::warn!(provider = %provider_name, op, error = %e, "Request failed, aborting");
                        return AttemptRun {
                            result: Some(Err(last_error.unwrap())),
                            attempts: attempt,
                            last_error: None,
                        };
                    }
                    tracing::warn!(provider = %provider_name, op, attempt, error = %e, "Request failed, trying next provider");

                    let backoff = e
                        .retry_after_ms()
                        .map(Duration::from_millis)
                        .unwrap_or_else(|| Duration::from_millis(200 * attempt as u64));
                    tokio::time::sleep(backoff).await;
                }
            }
        }

        AttemptRun {
            result: None,
            attempts: attempt,
            last_error,
        }
    }
}
