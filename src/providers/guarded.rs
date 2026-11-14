use std::sync::Arc;

use async_openai::types::models::Model;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use crate::providers::audio::{SpeechRequest, SpeechResponse, TranscriptionRequest, TranscriptionResponse};
use crate::providers::decision::{DecisionRequest, DecisionResponse};
use crate::providers::image::{ImageGenerationRequest, ImageResponse, ImageUploadRequest};
use crate::providers::{ChatRequest, Provider, ProviderError, StreamingChunk};
use crate::router::ModelRuntimeInfo;
use crate::ssrf::check_public_url;

pub struct GuardedProvider {
    inner: Arc<dyn Provider>,
    base_url: String,
}

impl GuardedProvider {
    pub fn wrap(inner: Arc<dyn Provider>, base_url: &str) -> Arc<dyn Provider> {
        Arc::new(Self { inner, base_url: base_url.to_string() })
    }

    async fn allow(&self) -> Result<(), ProviderError> {
        check_public_url(&self.base_url)
            .await
            .map_err(|e| ProviderError::Other(e.to_string().into()))
    }
}

#[async_trait]
impl Provider for GuardedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.allow().await?;
        self.inner.list_models().await
    }

    async fn chat_completions(&self, request: &ChatRequest) -> Result<crate::providers::ChatResponse, ProviderError> {
        self.allow().await?;
        self.inner.chat_completions(request).await
    }

    fn chat_completions_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        let inner = self.inner.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();
        let stream = async_stream::stream! {
            if let Err(e) = check_public_url(&base_url).await {
                yield Err(ProviderError::Other(e.to_string().into()));
                return;
            }
            match inner.chat_completions_stream(&request) {
                Ok(mut upstream) => {
                    while let Some(item) = upstream.next().await {
                        yield item;
                    }
                }
                Err(e) => yield Err(e),
            }
        };
        Ok(stream.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        self.allow().await?;
        self.inner.health_check().await
    }

    async fn responses(&self, request: &CreateResponse) -> Result<ApiResponse, ProviderError> {
        self.allow().await?;
        self.inner.responses(request).await
    }

    async fn responses_with_body(
        &self,
        request: &CreateResponse,
        body_defaults: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ApiResponse, ProviderError> {
        self.allow().await?;
        self.inner.responses_with_body(request, body_defaults).await
    }

    async fn transcriptions(&self, request: &TranscriptionRequest) -> Result<TranscriptionResponse, ProviderError> {
        self.allow().await?;
        self.inner.transcriptions(request).await
    }

    async fn speech(&self, request: &SpeechRequest) -> Result<SpeechResponse, ProviderError> {
        self.allow().await?;
        self.inner.speech(request).await
    }

    async fn list_voices(&self, model: &str) -> Result<Option<Vec<String>>, ProviderError> {
        self.allow().await?;
        self.inner.list_voices(model).await
    }

    async fn image_generations(&self, request: &ImageGenerationRequest) -> Result<ImageResponse, ProviderError> {
        self.allow().await?;
        self.inner.image_generations(request).await
    }

    async fn image_uploads(&self, request: &ImageUploadRequest) -> Result<ImageResponse, ProviderError> {
        self.allow().await?;
        self.inner.image_uploads(request).await
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        self.allow().await?;
        self.inner.decide(request).await
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        self.allow().await?;
        self.inner.get_runtime_info(model_id).await
    }

    fn reports_modalities(&self) -> bool {
        self.inner.reports_modalities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::openai::OpenAiProvider;

    fn guarded(url: &str) -> Arc<dyn Provider> {
        GuardedProvider::wrap(Arc::new(OpenAiProvider::new("p", Some("p"), url, Some("k"))), url)
    }

    #[tokio::test]
    async fn requests_to_a_private_host_never_leave() {
        let p = guarded("http://10.1.2.3:8080/v1");
        let err = p.list_models().await.unwrap_err().to_string();
        assert!(err.contains("non-public"), "{err}");
        assert!(p.health_check().await.is_err());
    }

    #[tokio::test]
    async fn rebinding_to_loopback_is_caught_on_every_call() {
        let p = guarded("http://localhost:1/v1");
        assert!(p.list_models().await.unwrap_err().to_string().contains("non-public"));
    }
}
