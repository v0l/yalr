use super::*;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// RoutstrProvider — wraps an OpenAI-compatible API and adds Routstr protocol
/// endpoints for balance tracking, making it cost-aware.
///
/// Routstr (RIP-01, RIP-08) is a protocol for LLM routers that accept Lightning
/// payments for inference. This provider type connects to upstream Routstr nodes
/// and polls their balance so the admin dashboard can show how much credit
/// remains with each upstream provider.
///
/// For more information on Routstr:
/// - RIP-01: https://github.com/lnbits/routstr/blob/main/RIP-01.md
/// - RIP-08: https://github.com/lnbits/routstr/blob/main/RIP-08.md
#[derive(Clone)]
pub struct RoutstrProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
    /// Optional API key for Authorization header (Bearer token)
    api_key: Option<String>,
    /// Cached balance in millisatoshis. Updated on health_check and periodically.
    balance_msat: Arc<AtomicI64>,
}

impl RoutstrProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let base_url = Url::parse(base_url).expect("Invalid Routstr base URL");

        Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: HttpClient::new(),
            base_url,
            api_key: api_key.map(String::from),
            balance_msat: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Fetch the current balance from the Routstr node's /v1/balance/info endpoint.
    pub(super) async fn fetch_balance_from_api(&self) -> Result<i64, ProviderError> {
        let balance_url = self
            .base_url
            .join("v1/balance/info")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(balance_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !resp.status().is_success() {
            return Err(ProviderError::ServerError {
                message: format!("Balance info returned {}", resp.status()),
                status_code: Some(resp.status().as_u16()),
            });
        }

        let body: serde_json::Value =
            resp.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        // Try multiple response formats for flexibility
        let balance = body
            .get("balance_msat")
            .and_then(|v| v.as_i64())
            .or_else(|| body.get("balance").and_then(|b| b.as_i64()))
            .or_else(|| body.get("data").and_then(|d| d.get("balance_msat")).and_then(|v| v.as_i64()))
            .ok_or_else(|| ProviderError::Other("Missing balance_msat in response".into()))?;

        Ok(balance)
    }

    /// Refresh the cached balance from the upstream Routstr node.
    ///
    /// Called during health checks and periodically. Updates the internal
    /// atomic cache that the dashboard reads.
    pub async fn refresh_balance(&self) {
        match self.fetch_balance_from_api().await {
            Ok(balance) => {
                self.balance_msat.store(balance, Ordering::Relaxed);
                tracing::debug!(
                    provider = self.name(),
                    balance_msat = balance,
                    "Routstr balance refreshed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    provider = self.name(),
                    error = %e,
                    "Failed to refresh Routstr balance"
                );
            }
        }
    }

    /// Get the cached balance in millisatoshis (fast, non-async read).
    pub fn cached_balance_msat(&self) -> i64 {
        self.balance_msat.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Provider for RoutstrProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        // Override to handle custom OpenRouter/OpenRouter-like formats
        let models_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(models_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            tracing::debug!(
                provider = self.name(),
                status = %response.status(),
                "Routstr GET /v1/models failed"
            );
            return Err(ProviderError::ServerError {
                message: format!("Failed to list models: {}", response.status()),
                status_code: Some(response.status().as_u16()),
            });
        }

        let body: serde_json::Value =
            response.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        let empty = Vec::new();
        let models = body
            .get("data")
            .and_then(|d| d.as_array())
            .unwrap_or(&empty);

        // Parse models flexibly - extract just the fields we need
        let mut result = Vec::new();
        for model in models {
            if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
                let owned_by = model
                    .get("owned_by")
                    .and_then(|v| v.as_str())
                    .unwrap_or("routstr")
                    .to_string();
                
                let created = model
                    .get("created")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as u32;

                result.push(Model {
                    id: id.to_string(),
                    object: "model".to_string(),
                    created,
                    owned_by,
                });
            }
        }

        Ok(result)
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
        // For Routstr providers, we do a simple connectivity check instead of
        // relying on the inner OpenAI provider's health check which may fail
        // to deserialize custom model formats.
        let health_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(health_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        match req.send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        // Fetch the models list from the Routstr node and find matching entry
        let models_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(models_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let response = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            tracing::debug!(
                provider = self.name(),
                status = %response.status(),
                "Routstr GET /v1/models failed"
            );
            return Ok(None);
        }

        let body: serde_json::Value =
            response.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        let empty = Vec::new();
        let models = body
            .get("data")
            .and_then(|d| d.as_array())
            .unwrap_or(&empty);

        let model_entry = models
            .iter()
            .find(|m| m.get("id").and_then(|id| id.as_str()) == Some(model_id));

        let mut additional_fields = std::collections::HashMap::new();
        if let Some(entry) = model_entry {
            if let Some(obj) = entry.as_object() {
                for (key, value) in obj {
                    if key != "id" {
                        additional_fields.insert(key.clone(), value.clone());
                    }
                }
            }
        }

        let context_length = model_entry
            .and_then(|m| m.get("context_length"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let max_concurrency = model_entry
            .and_then(|m| m.get("max_concurrency"))
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

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

    async fn fetch_balance(&self) -> Option<CurrencyAmount> {
        match self.fetch_balance_from_api().await {
            Ok(msats) => Some(CurrencyAmount::Msats(msats)),
            Err(e) => {
                tracing::warn!(
                    provider = self.name(),
                    error = %e,
                    "Failed to fetch balance from Routstr provider"
                );
                None
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ErrorType;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = RoutstrProvider::new(
            "Test Provider",
            Some("test"),
            "http://localhost:8080",
            Some("test-key"),
        );

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = RoutstrProvider::new(
            "My Provider",
            None,
            "http://localhost:8080",
            Some("key"),
        );
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = RoutstrProvider::new(
            "Test_Provider",
            Some("custom_slug"),
            "http://localhost:8080",
            Some("key"),
        );
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = RoutstrProvider::new(
            "Test",
            None,
            "http://localhost:8080",
            Some("my-api-key"),
        );
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = RoutstrProvider::new("Test", None, "http://localhost:8080", None);
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = RoutstrProvider::new(
            "Test",
            None,
            "http://localhost:8080",
            Some("key"),
        );
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_cached_balance_default_zero() {
        let provider = RoutstrProvider::new(
            "Test",
            None,
            "http://localhost:8080",
            Some("key"),
        );
        assert_eq!(provider.cached_balance_msat(), 0);
        let balance = provider.fetch_balance().await;
        // fetch_balance will fail to reach the fake URL → returns None
        assert_eq!(balance, None);
    }

    #[tokio::test]
    async fn test_chat_completions_stream_error_handling() {
        let provider = RoutstrProvider::new(
            "Test",
            None,
            "http://invalid-url",
            Some("key"),
        );
        let request = CreateChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            ..Default::default()
        };
        let result = provider.chat_completions_stream(&request);
        assert!(result.is_ok());
    }
}
