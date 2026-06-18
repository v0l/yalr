//! Anthropic provider implementation using the async-anthropic crate.
//!
//! This provider translates between OpenAI-compatible chat completion types
//! (used by the router) and Anthropic's native Messages API format.
//!
//! Conversion logic is split across submodules:
//! - [`convert`]: OpenAI request -> Anthropic request (messages, tools).
//! - [`response`]: Anthropic response/errors -> OpenAI response.
//! - [`stream`]: Anthropic SSE events -> OpenAI streaming chunks.

mod convert;
mod response;
mod stream;

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use super::*;

use convert::{build_anthropic_request, build_client, convert_messages};
pub use convert::set_prompt_cache_enabled;
use response::{anthropic_response_to_openai, map_anthropic_error};

/// Anthropic-specific provider using the async-anthropic crate.
///
/// Anthropic uses the Messages API (not OpenAI-compatible chat completions),
/// so this provider translates between the two formats internally.
#[derive(Clone)]
pub struct AnthropicProvider {
    name: String,
    slug: String,
    client: async_anthropic::Client,
    /// Raw reqwest client for lightweight health checks
    http_client: reqwest::Client,
    /// Base URL for constructing health check URLs
    base_url: String,
    /// API key for health check auth header
    api_key: String,
    /// Cached model list
    models_cache: Arc<RwLock<Option<(Vec<Model>, Instant)>>>,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug = slug
            .unwrap_or(name)
            .to_lowercase()
            .replace(' ', "-")
            .replace('_', "-");

        let client = build_client(base_url, api_key);

        Self {
            name: name.to_string(),
            slug,
            client,
            http_client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.unwrap_or("").to_string(),
            models_cache: Arc::new(RwLock::new(None)),
        }
    }

}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        // Return cached models if fresh (< 5 min)
        if let Some((ref cached, cached_at)) = *self.models_cache.read().await {
            if cached_at.elapsed() < std::time::Duration::from_secs(300) {
                return Ok(cached.clone());
            }
        }
        let response = self
            .client
            .models()
            .list()
            .await
            .map_err(map_anthropic_error)?;

        let models: Vec<Model> = response
            .data
            .into_iter()
            .map(|m| Model {
                id: m.id,
                object: "model".to_string(),
                created: 0,
                owned_by: m.display_name,
            })
            .collect();

        *self.models_cache.write().await = Some((models.clone(), Instant::now()));
        Ok(models)
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let (system, messages) = convert_messages(&request.messages);
        let anthropic_request = build_anthropic_request(system, messages, request);

        let response = self
            .client
            .messages()
            .create(anthropic_request)
            .await
            .map_err(map_anthropic_error)?;

        Ok(anthropic_response_to_openai(&response, &request.model))
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        stream::chat_completions_stream(self.client.clone(), request)
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        // Lightweight GET to /v1/models — just check if the server responds.
        // Append /v1/models if base URL doesn't already end with /v1
        let base = self.base_url.trim_end_matches('/');
        let url = if base.ends_with("/v1") {
            format!("{}/models", base)
        } else {
            format!("{}/v1/models", base)
        };
        let mut req = self.http_client.get(&url).header("anthropic-version", "2023-06-01");
        if !self.api_key.is_empty() {
            req = req.header("x-api-key", &self.api_key);
        } else {
            tracing::warn!(provider = %self.name(), "No API key configured for Anthropic provider");
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    tracing::warn!(
                        provider = %self.name(),
                        url = %url,
                        status = %status,
                        "Health check returned non-success status"
                    );
                }
                Ok(status.is_success())
            },
            Err(e) => {
                tracing::warn!(
                    provider = %self.name(),
                    url = %url,
                    error = %e,
                    "Health check request failed"
                );
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            Some("anthropic"),
            "https://api.anthropic.com",
            Some("test-key"),
        );
        assert_eq!(provider.name(), "Anthropic");
        assert_eq!(provider.slug(), "anthropic");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider = AnthropicProvider::new(
            "My Provider",
            None,
            "https://api.anthropic.com",
            Some("key"),
        );
        assert_eq!(provider.slug(), "my-provider");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = AnthropicProvider::new("Anthropic", None, "https://api.anthropic.com", None);
        assert_eq!(provider.name(), "Anthropic");
    }

    #[tokio::test]
    async fn test_chat_completions_invalid_url() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            None,
            "https://invalid.anthropic.example.com",
            Some("test-key"),
        );

        let request = CreateChatCompletionRequest {
            model: "claude-3-haiku-20240307".to_string(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("Hello".to_string()),
                    name: None,
                },
            )],
            ..Default::default()
        };

        let result = provider.chat_completions(&request).await;
        assert!(result.is_err());
    }
}
