use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::MetricsUser;
use crate::providers::audio::{
    SpeechRequest, SpeechResponse, TranscriptionRequest, TranscriptionResponse,
};
use crate::providers::Provider;
use crate::router::engine::{InFlightGuard, Router, RouterError};
use crate::ProviderError;

/// Outcome of one provider attempt, before metrics are recorded.
type Attempt<T> = Result<T, ProviderError>;

/// What one pass over a candidate list produced. `attempts` counts real
/// attempts, so zero means every candidate lacked audio support entirely.
struct AttemptRun<T> {
    result: Option<Result<T, RouterError>>,
    attempts: u32,
    last_error: Option<RouterError>,
}

impl Router {
    pub async fn transcriptions(
        &self,
        request: &TranscriptionRequest,
        user: Option<MetricsUser>,
    ) -> Result<TranscriptionResponse, RouterError> {
        let label = if request.translate {
            "audio.translations"
        } else {
            "audio.transcriptions"
        };
        self.audio_failover(&request.model, label, user, |provider, model| {
            let mut request = request.clone();
            request.model = model;
            async move { provider.transcriptions(&request).await }
        })
        .await
    }

    pub async fn speech(
        &self,
        request: &SpeechRequest,
        user: Option<MetricsUser>,
    ) -> Result<SpeechResponse, RouterError> {
        self.audio_failover(&request.model, "audio.speech", user, |provider, model| {
            let mut request = request.clone();
            request.model = model;
            async move { provider.speech(&request).await }
        })
        .await
    }

    /// Try each candidate backend for `model` in routing order, recording
    /// metrics per attempt and failing over on transient errors.
    ///
    /// `ProviderError::Unsupported` means the backend has no audio endpoint at
    /// all. A mixed chat/audio pool is the normal configuration, so that is
    /// neither a failure event nor a used-up retry: the candidate is skipped
    /// silently and its health is left alone.
    async fn audio_failover<T, F, Fut>(
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
        // list, hiding an audio provider that is merely degraded. Capability
        // beats health here: if nothing in the healthy set could serve audio at
        // all, try the configured backends that were filtered out.
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
                    "No healthy provider supports audio, falling back to filtered providers"
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
                        "Audio request completed successfully"
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
                        "Provider cannot serve audio requests, skipping"
                    );
                    if last_error.is_none() {
                        last_error = Some(RouterError::ProviderError(ProviderError::Unsupported(
                            message,
                        )));
                    }
                }
                Err(e) => {
                    attempt += 1;
                    self.metrics_store.emitter().emit_failure_with_details(
                        &provider_name,
                        model,
                        e.error_type(),
                        None,
                        &e.to_string(),
                        e.retry_after_ms(),
                        e.status_code(),
                        user.clone(),
                    );

                    last_error = Some(RouterError::ProviderError(e.clone()));

                    if !e.is_transient() {
                        tracing::warn!(provider = %provider_name, op, error = %e, "Audio request failed, aborting");
                        return AttemptRun {
                            result: Some(Err(last_error.unwrap())),
                            attempts: attempt,
                            last_error: None,
                        };
                    }
                    tracing::warn!(provider = %provider_name, op, attempt, error = %e, "Audio request failed, trying next provider");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::MetricsStore;
    use crate::providers::audio::audio_mime_for_format;
    use async_trait::async_trait;
    use bytes::Bytes;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeAudioProvider {
        name: String,
        behaviour: Behaviour,
        calls: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    enum Behaviour {
        Ok,
        Unsupported,
        Transient,
        Auth,
    }

    #[async_trait]
    impl Provider for FakeAudioProvider {
        fn name(&self) -> &str {
            &self.name
        }
        fn slug(&self) -> &str {
            &self.name
        }
        async fn list_models(&self) -> Result<Vec<crate::providers::Model>, ProviderError> {
            Ok(vec![])
        }
        async fn chat_completions(
            &self,
            _request: &crate::providers::ChatRequest,
        ) -> Result<crate::ChatCompletionResponse, ProviderError> {
            Err(ProviderError::Timeout)
        }
        fn chat_completions_stream(
            &self,
            _request: &crate::providers::ChatRequest,
        ) -> Result<
            futures::stream::BoxStream<
                'static,
                Result<crate::providers::StreamingChunk, ProviderError>,
            >,
            ProviderError,
        > {
            Err(ProviderError::Timeout)
        }
        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
        async fn transcriptions(
            &self,
            _request: &TranscriptionRequest,
        ) -> Result<TranscriptionResponse, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.behaviour {
                Behaviour::Ok => Ok(TranscriptionResponse {
                    content_type: "application/json".into(),
                    body: Bytes::from_static(br#"{"text":"ok"}"#),
                }),
                Behaviour::Unsupported => Err(ProviderError::Unsupported("no audio".into())),
                Behaviour::Transient => Err(ProviderError::Timeout),
                Behaviour::Auth => Err(ProviderError::Authentication("bad key".into())),
            }
        }
    }

    async fn router_with(providers: Vec<Arc<dyn Provider>>) -> Router {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let router = Router::new(MetricsStore::new(100), db);
        router.register_route("whisper-1", providers).await;
        router
    }

    fn fake(name: &str, behaviour: Behaviour) -> (Arc<dyn Provider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(FakeAudioProvider {
                name: name.to_string(),
                behaviour,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    fn request() -> TranscriptionRequest {
        TranscriptionRequest {
            model: "whisper-1".into(),
            file_name: "a.wav".into(),
            file: Bytes::from_static(b"x"),
            content_type: None,
            language: None,
            prompt: None,
            response_format: None,
            temperature: None,
            timestamp_granularities: vec![],
            translate: false,
        }
    }

    #[tokio::test]
    async fn skips_provider_without_audio_support() {
        let (no_audio, skipped) = fake("no-audio", Behaviour::Unsupported);
        let (good, used) = fake("whisper", Behaviour::Ok);
        let router = router_with(vec![no_audio, good]).await;

        let response = router.transcriptions(&request(), None).await.unwrap();
        assert_eq!(response.text().as_deref(), Some("ok"));
        assert_eq!(skipped.load(Ordering::SeqCst), 1);
        assert_eq!(used.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unsupported_does_not_degrade_provider_health() {
        let (no_audio, _) = fake("no-audio", Behaviour::Unsupported);
        let (good, _) = fake("whisper", Behaviour::Ok);
        let router = router_with(vec![no_audio, good]).await;

        router.transcriptions(&request(), None).await.unwrap();

        assert_eq!(router.metrics_store.get_recent_failures("no-audio").await, 0);
        assert_eq!(
            router.metrics_store.get_provider_health("no-audio").await,
            crate::metrics::HealthState::Healthy
        );
        assert_eq!(
            router.metrics_store.get_provider_backoff("no-audio").await,
            std::time::Duration::ZERO
        );
    }

    #[tokio::test]
    async fn unsupported_providers_do_not_consume_the_retry_budget() {
        let mut providers: Vec<Arc<dyn Provider>> = (0..5)
            .map(|i| fake(&format!("chat-only-{i}"), Behaviour::Unsupported).0)
            .collect();
        let (good, used) = fake("whisper", Behaviour::Ok);
        providers.push(good);
        let router = router_with(providers).await;

        assert!(router.transcriptions(&request(), None).await.is_ok());
        assert_eq!(used.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unhealthy_audio_provider_beats_a_healthy_chat_only_one() {
        let (chat_only, skipped) = fake("chat-only", Behaviour::Unsupported);
        let (audio, used) = fake("whisper", Behaviour::Ok);
        let router = router_with(vec![chat_only, audio]).await;

        // Health filtering drops the audio provider from the candidate list,
        // leaving a chat-only backend that cannot serve the request at all.
        for _ in 0..6 {
            router.metrics_store.emitter().emit_failure(
                "whisper",
                "whisper-1",
                crate::metrics::ErrorType::ServerError,
                "down",
                None,
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            router.metrics_store.get_provider_health("whisper").await,
            crate::metrics::HealthState::Unhealthy
        );

        let response = router.transcriptions(&request(), None).await.unwrap();
        assert_eq!(response.text().as_deref(), Some("ok"));
        assert_eq!(skipped.load(Ordering::SeqCst), 1);
        assert_eq!(used.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn all_unsupported_reports_the_capability_error() {
        let (a, _) = fake("chat-a", Behaviour::Unsupported);
        let (b, _) = fake("chat-b", Behaviour::Unsupported);
        let router = router_with(vec![a, b]).await;

        let err = router.transcriptions(&request(), None).await.unwrap_err();
        assert!(
            matches!(err, RouterError::ProviderError(ProviderError::Unsupported(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn fails_over_on_transient_error() {
        let (flaky, tried) = fake("flaky", Behaviour::Transient);
        let (good, used) = fake("whisper", Behaviour::Ok);
        let router = router_with(vec![flaky, good]).await;

        assert!(router.transcriptions(&request(), None).await.is_ok());
        assert_eq!(tried.load(Ordering::SeqCst), 1);
        assert_eq!(used.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn auth_error_aborts_without_trying_next() {
        let (bad, tried) = fake("bad-key", Behaviour::Auth);
        let (good, untouched) = fake("whisper", Behaviour::Ok);
        let router = router_with(vec![bad, good]).await;

        assert!(router.transcriptions(&request(), None).await.is_err());
        assert_eq!(tried.load(Ordering::SeqCst), 1);
        assert_eq!(untouched.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unknown_model_has_no_provider() {
        let router = router_with(vec![]).await;
        let mut req = request();
        req.model = "nope".into();
        assert!(matches!(
            router.transcriptions(&req, None).await,
            Err(RouterError::NoAvailableProvider)
        ));
    }

    #[test]
    fn speech_mime_fallback_is_shared() {
        assert_eq!(audio_mime_for_format(Some("opus")), "audio/opus");
    }
}
