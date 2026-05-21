use super::*;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// VllmProvider - A wrapper around OpenAiProvider with vLLM-specific runtime info.
///
/// vLLM provides an OpenAI-compatible API but does NOT support
/// `GET /v1/models/{model_id}` (the retrieve endpoint). Instead we fetch
/// `GET /v1/models` and find the matching entry.
///
/// For more information on vLLM:
/// - API Documentation: https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html
#[derive(Clone)]
pub struct VllmProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
}

impl VllmProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let base_url = Url::parse(base_url).expect("Invalid vLLM base URL");

        Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: HttpClient::new(),
            base_url,
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

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        // vLLM doesn't support GET /v1/models/{id}, so we fetch the full
        // model list and find the matching entry.
        let models_url = self.base_url.join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let response = self.http_client
            .get(models_url.as_str())
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            tracing::debug!(
                provider = self.name(),
                status = %response.status(),
                "vLLM /v1/models request failed"
            );
            return Ok(None);
        }

        let body: ModelListResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        // Find the model in the list
        let model_entry = body.data.iter().find(|m| m.id == model_id);

        let context_length = model_entry.and_then(|m| m.context_length);
        let max_concurrency = model_entry.and_then(|m| m.max_concurrency);

        let mut additional_fields = std::collections::HashMap::new();
        if let Some(entry) = model_entry {
            additional_fields = entry.extra.clone();
            // id is already stored as model_id, drop it from extras
            additional_fields.remove("id");
        }

        let runtime_info = ModelRuntimeInfo {
            model_id: model_id.to_string(),
            context_length,
            quantization: None,
            variant: None,
            parameter_size: None,
            max_output_tokens: None,
            max_concurrency,
            modalities: vec![Modality::Text],
            additional_fields,
        };

        Ok(Some(runtime_info))
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
