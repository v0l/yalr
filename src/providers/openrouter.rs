use super::*;
use async_openai::config::OpenAIConfig;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use crate::router::{Modality, ModelRuntimeInfo};

/// Response from OpenRouter's /v1/credits API endpoint
#[derive(Debug, Clone, Deserialize)]
struct CreditsResponse {
    data: CreditsData,
}

/// Credits data structure from OpenRouter API
#[derive(Debug, Clone, Deserialize)]
struct CreditsData {
    /// Total credits purchased
    total_credits: f64,
    /// Total credits used
    total_usage: f64,
}

/// OpenRouterProvider wraps an OpenAI-compatible client and adds OpenRouter-specific
/// features like balance tracking via the /v1/credits endpoint.
///
/// OpenRouter (https://openrouter.ai/) provides access to multiple LLM providers
/// through a unified API. This provider type adds support for fetching credit
/// balance information from OpenRouter's management API.
#[derive(Clone)]
pub struct OpenRouterProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: String,
    /// API key stored separately for balance API access
    api_key: Option<String>,
    models_cache: ModelsCache,
}

impl OpenRouterProvider {
    /// Create a new OpenRouter provider instance
    /// 
    /// # Arguments
    /// * `name` - Human-readable name for the provider
    /// * `slug` - Unique slug identifier (defaults to lowercase name with hyphens)
    /// * `base_url` - OpenRouter API base URL (typically "https://openrouter.ai/api/v1")
    /// * `api_key` - OpenRouter API key (management key required for balance API)
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug_str = slug.unwrap_or(name).to_lowercase().replace(" ", "-").replace("_", "-");
        
        // Strip trailing slash to avoid double slashes in API URLs
        let base_url = base_url.trim_end_matches('/');
        
        let config = OpenAIConfig::default()
            .with_api_base(base_url)
            .with_api_key(api_key.unwrap_or(""));
        
        Self {
            inner: OpenAiProvider::new(name, Some(&slug_str), base_url, api_key),
            http_client: HttpClient::new(),
            base_url: base_url.to_string(),
            api_key: api_key.map(String::from),
            models_cache: ModelsCache::new(),
        }
    }

    /// Fetch balance from OpenRouter's /v1/credits endpoint
    /// 
    /// Returns the remaining balance in USD microcents (µ$).
    /// Requires a valid management API key for authentication.
    /// 
    /// # Returns
    /// * `Some(CurrencyAmount::UsdMicro(balance))` - Remaining balance in microcents
    /// * `None` - If API key is missing, request fails, or response is invalid
    pub(super) async fn fetch_balance_from_api(&self) -> Option<CurrencyAmount> {
        let api_key = self.api_key.as_ref()?;
        if api_key.is_empty() {
            tracing::debug!("No API key configured for OpenRouter balance fetch");
            return None;
        }

        let credits_url = format!("{}/credits", self.base_url);
        
        match self.http_client.get(&credits_url).bearer_auth(api_key).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    tracing::warn!(
                        provider = %self.name(),
                        status = %response.status(),
                        "OpenRouter credits API request failed"
                    );
                    return None;
                }

                match response.json::<CreditsResponse>().await {
                    Ok(credits_response) => {
                        let remaining = credits_response.data.total_credits - credits_response.data.total_usage;
                        // Convert USD to microcents (multiply by 1,000,000)
                        let usd_micro = (remaining * 1_000_000.0) as i64;
                        tracing::debug!(
                            provider = %self.name(),
                            total_credits = credits_response.data.total_credits,
                            total_usage = credits_response.data.total_usage,
                            remaining_usd_micro = usd_micro,
                            "Fetched balance from OpenRouter credits API"
                        );
                        Some(CurrencyAmount::UsdMicro(usd_micro))
                    }
                    Err(e) => {
                        tracing::warn!(
                            provider = %self.name(),
                            error = %e,
                            "Failed to parse OpenRouter credits response"
                        );
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    provider = %self.name(),
                    error = %e,
                    "Failed to fetch balance from OpenRouter credits API"
                );
                None
            }
        }
    }
}

#[async_trait::async_trait]
impl Provider for OpenRouterProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        if let Some(cached) = self.models_cache.get().await {
            return Ok(cached);
        }

        // Use custom OpenRouter model format instead of OpenAI's standard format
        let models_url = format!("{}/models", self.base_url.trim_end_matches('/'));
        
        let mut req = self.http_client.get(&models_url);
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }
        
        let response = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;
        
        if !response.status().is_success() {
            return Err(ProviderError::ServerError {
                message: format!("Failed to list models: {}", response.status()),
                status_code: Some(response.status().as_u16()),
            });
        }
        
        let body: ModelListResponse = response.json().await.map_err(|e| ProviderError::Other(e.into()))?;
        
        // Parse OpenRouter models and convert to standard Model type
        let models: Vec<Model> = body.data.into_iter().map(|item| Model {
            id: item.id,
            object: item.object.unwrap_or_else(|| "model".to_string()),
            created: item.created.unwrap_or(0),
            owned_by: item.owned_by.unwrap_or_else(|| "openrouter".to_string()),
        }).collect();

        self.models_cache.store(models.clone()).await;
        Ok(models)
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

    async fn responses(&self, request: &CreateResponse) -> Result<ApiResponse, ProviderError> {
        self.inner.responses(request).await
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        self.inner.get_runtime_info(model_id).await
    }

    async fn fetch_balance(&self) -> Option<CurrencyAmount> {
        self.fetch_balance_from_api().await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = OpenRouterProvider::new(
            "OpenRouter",
            Some("openrouter"),
            "https://openrouter.ai/api/v1",
            Some("test-key"),
        );

        assert_eq!(provider.name(), "OpenRouter");
        assert_eq!(provider.slug(), "openrouter");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = OpenRouterProvider::new(
            "My Provider",
            None,
            "https://openrouter.ai/api/v1",
            Some("key"),
        );
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = OpenRouterProvider::new(
            "Test_Provider",
            Some("custom_slug"),
            "https://openrouter.ai/api/v1",
            Some("key"),
        );
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = OpenRouterProvider::new(
            "OpenRouter",
            None,
            "https://openrouter.ai/api/v1",
            Some("my-api-key"),
        );
        assert_eq!(provider.name(), "OpenRouter");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = OpenRouterProvider::new(
            "OpenRouter",
            None,
            "https://openrouter.ai/api/v1",
            None,
        );
        assert_eq!(provider.name(), "OpenRouter");
    }

    #[tokio::test]
    async fn test_fetch_balance_returns_none_without_api_key() {
        let provider = OpenRouterProvider::new(
            "OpenRouter",
            None,
            "https://openrouter.ai/api/v1",
            None,
        );
        let balance = provider.fetch_balance().await;
        assert!(balance.is_none());
    }

    #[tokio::test]
    async fn test_credits_response_deserialization() {
        let json_str = r#"{
            "data": {
                "total_credits": 100.0,
                "total_usage": 25.5
            }
        }"#;
        
        let response: CreditsResponse = serde_json::from_str(json_str).unwrap();
        assert_eq!(response.data.total_credits, 100.0);
        assert_eq!(response.data.total_usage, 25.5);
    }

    #[tokio::test]
    async fn test_credits_balance_calculation() {
        let json_str = r#"{
            "data": {
                "total_credits": 100.0,
                "total_usage": 25.5
            }
        }"#;
        
        let response: CreditsResponse = serde_json::from_str(json_str).unwrap();
        let remaining = response.data.total_credits - response.data.total_usage;
        let usd_micro = (remaining * 1_000_000.0) as i64;
        
        assert_eq!(remaining, 74.5);
        assert_eq!(usd_micro, 74_500_000);
    }
}
