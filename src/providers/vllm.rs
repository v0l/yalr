use super::*;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;

/// VllmProvider - A wrapper around OpenAiProvider for vLLM servers.
/// 
/// vLLM provides a 100% OpenAI-compatible API, so we reuse the OpenAiProvider
/// implementation and just change the name/slug for identification.
/// 
/// For more information on vLLM:
/// - API Documentation: https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html
#[derive(Clone)]
pub struct VllmProvider {
    inner: OpenAiProvider,
}

impl VllmProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        Self {
            inner: OpenAiProvider::new(name, slug, base_url, api_key),
        }
    }
}

#[async_trait::async_trait]
impl Provider for VllmProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.inner.list_models().await
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        self.inner.chat_completions(request).await
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        self.inner.chat_completions_stream(request)
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        self.inner.health_check().await
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<crate::router::ModelRuntimeInfo>, ProviderError> {
        self.inner.get_runtime_info(model_id).await
    }

    async fn responses(
        &self,
        request: &CreateResponse,
    ) -> Result<ApiResponse, ProviderError> {
        self.inner.responses(request).await
    }
}

#[cfg(test)]
mod tests {
use super::*;
use futures::stream::BoxStream;
use crate::router::ModelRuntimeInfo;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = VllmProvider::new("Test Provider", Some("test"), "http://localhost:8080", Some("test-key"));

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = VllmProvider::new("My Provider", None, "http://localhost:8080", Some("key"));
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = VllmProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:8080", Some("key"));
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", Some("my-api-key"));
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", None);
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", Some("key"));
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_list_models_error_handling() {
        let provider = VllmProvider::new("Test", None, "http://invalid-url", Some("key"));
        let result = provider.list_models().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_completions_stream_error_handling() {
        let provider = VllmProvider::new("Test", None, "http://invalid-url", Some("key"));
        let request = CreateChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            ..Default::default()
        };
        let result = provider.chat_completions_stream(&request);
        assert!(result.is_ok());
    }
}
