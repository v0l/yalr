use std::sync::Arc;

use crate::metrics::MetricsStore;
use crate::providers::Provider;
use crate::router::Modality;
use crate::ProviderError;

pub(crate) async fn missing_input(
    metrics: &MetricsStore,
    provider: &Arc<dyn Provider>,
    model: &str,
    needed: &[Modality],
) -> Option<Modality> {
    if needed.is_empty() || !provider.reports_modalities() {
        return None;
    }
    if !metrics.has_provider_runtime_info(provider.name()).await {
        if let Ok(Some(info)) = provider.get_runtime_info(model).await {
            metrics.set_provider_runtime_info(provider.name(), info).await;
        }
    }
    let supported = metrics.get_provider_modalities(provider.name()).await?;
    needed.iter().find(|m| !supported.contains(m)).copied()
}

pub(crate) fn skipped_error(provider: &str, modality: Modality) -> ProviderError {
    ProviderError::Unsupported(format!(
        "{provider} does not accept {} input",
        modality.as_str()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::router::engine::Router;
    use crate::router::ModelRuntimeInfo;
    use crate::{ChatRequest, ChatResponse, RouterError};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Backend {
        name: String,
        vision: Option<bool>,
        unsupported: bool,
        calls: Arc<AtomicUsize>,
    }

    fn response() -> ChatResponse {
        serde_json::from_str(
            r#"{"id":"c","object":"chat.completion","created":1,"model":"m",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}"#,
        )
        .unwrap()
    }

    fn chunk() -> crate::StreamingChunk {
        serde_json::from_str(
            r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m",
            "choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
        )
        .unwrap()
    }

    #[async_trait]
    impl Provider for Backend {
        fn name(&self) -> &str {
            &self.name
        }
        fn slug(&self) -> &str {
            &self.name
        }
        async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
            Ok(vec![])
        }
        async fn chat_completions(&self, _request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.unsupported {
                return Err(ProviderError::Unsupported("no audio".into()));
            }
            Ok(response())
        }
        fn chat_completions_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>,
            ProviderError,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.unsupported {
                return Err(ProviderError::Unsupported("no audio".into()));
            }
            Ok(Box::pin(futures::stream::once(async { Ok(chunk()) })))
        }
        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
        fn reports_modalities(&self) -> bool {
            self.vision.is_some()
        }
        async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
            let Some(vision) = self.vision else {
                return Ok(None);
            };
            let mut modalities = vec![Modality::Text];
            if vision {
                modalities.push(Modality::Image);
            }
            Ok(Some(ModelRuntimeInfo {
                model_id: model_id.to_string(),
                context_length: None,
                quantization: None,
                variant: None,
                parameter_size: None,
                max_output_tokens: None,
                max_concurrency: None,
                modalities,
                additional_fields: Default::default(),
            }))
        }
    }

    fn backend(name: &str, vision: Option<bool>, unsupported: bool) -> (Arc<dyn Provider>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(Backend {
                name: name.into(),
                vision,
                unsupported,
                calls: calls.clone(),
            }),
            calls,
        )
    }

    async fn router(backends: Vec<Arc<dyn Provider>>) -> Router {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let router = Router::new(MetricsStore::new(100), db);
        router.register_route("pool", backends).await;
        router
    }

    fn image_request() -> ChatRequest {
        serde_json::from_str(
            r#"{"model":"pool","messages":[{"role":"user","content":[{"type":"text","text":"what"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}}]}]}"#,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn text_only_backend_is_skipped_for_an_image() {
        let (text, text_calls) = backend("text", Some(false), false);
        let (vision, vision_calls) = backend("vision", Some(true), false);
        let router = router(vec![text, vision]).await;

        for _ in 0..2 {
            router.chat_completions(&image_request(), None).await.unwrap();
        }
        assert_eq!(text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(vision_calls.load(Ordering::SeqCst), 2);
        assert_eq!(router.metrics_store.get_recent_failures("text").await, 0);
    }

    #[tokio::test]
    async fn streaming_skips_a_text_only_backend_too() {
        use futures::StreamExt;
        let (text, text_calls) = backend("text", Some(false), false);
        let (vision, vision_calls) = backend("vision", Some(true), false);
        let router = router(vec![text, vision]).await;

        let mut stream = router.chat_completions_stream(&image_request(), None).await.unwrap();
        while let Some(chunk) = stream.next().await {
            chunk.unwrap();
        }
        assert_eq!(text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(vision_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unsupported_backend_is_passed_over_without_a_failure() {
        let (claude, claude_calls) = backend("claude", None, true);
        let (gpt, gpt_calls) = backend("gpt", None, false);
        let router = router(vec![claude, gpt]).await;

        router.chat_completions(&image_request(), None).await.unwrap();
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);
        assert_eq!(gpt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(router.metrics_store.get_recent_failures("claude").await, 0);
    }

    #[tokio::test]
    async fn no_capable_backend_reports_unsupported() {
        let (text, _) = backend("text", Some(false), false);
        let router = router(vec![text]).await;

        let err = router.chat_completions(&image_request(), None).await.unwrap_err();
        assert!(
            matches!(err, RouterError::ProviderError(ProviderError::Unsupported(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn backend_that_cannot_report_is_tried() {
        let (unknown, calls) = backend("unknown", None, false);
        let router = router(vec![unknown]).await;

        router.chat_completions(&image_request(), None).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
