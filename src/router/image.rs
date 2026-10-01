use crate::metrics::MetricsUser;
use crate::providers::image::{ImageGenerationRequest, ImageResponse, ImageUploadRequest};
use crate::router::engine::{Router, RouterError};

impl Router {
    pub async fn image_generations(
        &self,
        request: &ImageGenerationRequest,
        user: Option<MetricsUser>,
    ) -> Result<ImageResponse, RouterError> {
        self.capability_failover(&request.model, "images.generations", user, |provider, model| {
            let mut request = request.clone();
            request.model = model;
            async move { provider.image_generations(&request).await }
        })
        .await
    }

    pub async fn image_uploads(
        &self,
        request: &ImageUploadRequest,
        user: Option<MetricsUser>,
    ) -> Result<ImageResponse, RouterError> {
        self.capability_failover(&request.model, request.operation(), user, |provider, model| {
            let mut request = request.clone();
            request.model = model;
            async move { provider.image_uploads(&request).await }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::MetricsStore;
    use crate::providers::image::ImageUploadKind;
    use crate::providers::Provider;
    use crate::ProviderError;
    use async_trait::async_trait;
    use bytes::Bytes;
    use std::sync::{Arc, Mutex};

    struct FakeImager {
        name: String,
        supported: bool,
        seen_models: Arc<Mutex<Vec<String>>>,
    }

    impl FakeImager {
        fn answer(&self, model: &str) -> Result<ImageResponse, ProviderError> {
            self.seen_models.lock().unwrap().push(model.to_string());
            if !self.supported {
                return Err(ProviderError::Unsupported("chat only".into()));
            }
            Ok(ImageResponse {
                content_type: "application/json".into(),
                stream: Box::pin(futures::stream::once(async {
                    Ok(Bytes::from_static(br#"{"data":[]}"#))
                })),
            })
        }
    }

    #[async_trait]
    impl Provider for FakeImager {
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
            futures::stream::BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
            ProviderError,
        > {
            Err(ProviderError::Timeout)
        }
        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
        async fn image_generations(
            &self,
            request: &ImageGenerationRequest,
        ) -> Result<ImageResponse, ProviderError> {
            self.answer(&request.model)
        }
        async fn image_uploads(
            &self,
            request: &ImageUploadRequest,
        ) -> Result<ImageResponse, ProviderError> {
            self.answer(&request.model)
        }
    }

    fn fake(name: &str, supported: bool) -> (Arc<dyn Provider>, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(FakeImager {
                name: name.into(),
                supported,
                seen_models: seen.clone(),
            }),
            seen,
        )
    }

    async fn router() -> Router {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        Router::new(MetricsStore::new(100), db)
    }

    fn generation(model: &str) -> ImageGenerationRequest {
        serde_json::from_str(&format!(r#"{{"model":"{model}","prompt":"a cat"}}"#)).unwrap()
    }

    #[tokio::test]
    async fn generation_skips_backend_without_images() {
        let (chat, chat_seen) = fake("chat", false);
        let (imager, imager_seen) = fake("imager", true);
        let router = router().await;
        router.register_route("gpt-image-1", vec![chat, imager]).await;

        assert!(router.image_generations(&generation("gpt-image-1"), None).await.is_ok());
        assert_eq!(chat_seen.lock().unwrap().len(), 1);
        assert_eq!(imager_seen.lock().unwrap().len(), 1);
        assert_eq!(router.metrics_store.get_recent_failures("chat").await, 0);
    }

    #[tokio::test]
    async fn prefixed_upload_reaches_the_provider_without_its_slug() {
        let (openai, seen) = fake("openai", true);
        let router = router().await;
        router.add_provider(openai).await;

        let request = ImageUploadRequest {
            kind: ImageUploadKind::Edit,
            model: "openai/gpt-image-1".into(),
            files: vec![],
            fields: vec![],
        };
        assert!(router.image_uploads(&request, None).await.is_ok());
        assert_eq!(*seen.lock().unwrap(), vec!["gpt-image-1".to_string()]);
    }

    #[tokio::test]
    async fn no_image_backend_reports_unsupported() {
        let (chat, _) = fake("chat", false);
        let router = router().await;
        router.register_route("gpt-image-1", vec![chat]).await;

        let err = router
            .image_generations(&generation("gpt-image-1"), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RouterError::ProviderError(ProviderError::Unsupported(_))),
            "got {err:?}"
        );
    }
}
