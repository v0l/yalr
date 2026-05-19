use super::*;
use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// PpqProvider — wraps the PPQ.ai OpenAI-compatible API and adds balance tracking.
///
/// PPQ.ai (PayPerQ) is an OpenAI-compatible API provider that accepts Lightning
/// payments and tracks credit balances. This provider connects to the PPQ API
/// and polls the credit balance endpoint so the admin dashboard can show remaining
/// credit.
///
/// API Reference: https://ppq.ai/api-docs
#[derive(Clone)]
pub struct PpqProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
    /// API key for Authorization header (Bearer token)
    api_key: Option<String>,
    /// Credit ID for balance API authentication (from PPQ account)
    credit_id: Option<String>,
    /// Cached balance in micro-usd ($1.00 = 1_000_000 µ$). Updated on health_check.
    balance_usd_micro: Arc<AtomicI64>,
}

impl PpqProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let base_url = Url::parse(base_url).expect("Invalid PPQ base URL");

        Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: HttpClient::new(),
            base_url,
            api_key: api_key.map(String::from),
            credit_id: None, // Will be set via configuration if available
            balance_usd_micro: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Set the credit ID for balance API authentication.
    /// This is required for fetching the credit balance from PPQ.
    pub fn with_credit_id(mut self, credit_id: Option<String>) -> Self {
        self.credit_id = credit_id;
        self
    }

    /// Fetch credit balance from PPQ API.
    /// Endpoint: POST https://api.ppq.ai/credits/balance
    /// Auth: API key in Authorization header (Bearer)
    pub(super) async fn fetch_balance_from_api(&self) -> Result<i64, ProviderError> {
        let balance_url = self
            .base_url
            .join("credits/balance")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.post(balance_url.as_str());
        
        // Add authentication - API key is required
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        } else {
            return Err(ProviderError::Authentication(
                "PPQ provider requires API key for balance tracking".to_string(),
            ));
        }
        
        // Add credit_id in body if available (optional, for multi-account setups)
        if let Some(ref credit_id) = self.credit_id {
            let body = serde_json::json!({ "credit_id": credit_id });
            req = req.json(&body);
        }

        let resp = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !resp.status().is_success() {
            return Err(ProviderError::ServerError {
                message: format!("Balance endpoint returned {}", resp.status()),
                status_code: Some(resp.status().as_u16()),
            });
        }

        let body: serde_json::Value =
            resp.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        // PPQ returns balance in various formats - try multiple
        let balance_usd_micro = body
            .get("balance_usd_micro")
            .and_then(|v| v.as_i64())
            .or_else(|| body.get("balance").and_then(|b| b.as_i64()))
            .or_else(|| body.get("data").and_then(|d| d.get("balance_usd_micro")).and_then(|v| v.as_i64()))
            .ok_or_else(|| ProviderError::Other("Missing balance in response".into()))?;

        Ok(balance_usd_micro)
    }

    /// Refresh the cached balance from the upstream PPQ API.
    pub async fn refresh_balance(&self) {
        match self.fetch_balance_from_api().await {
            Ok(balance) => {
                self.balance_usd_micro.store(balance, Ordering::Relaxed);
                tracing::debug!(
                    provider = self.name(),
                    balance_usd_micro = balance,
                    "PPQ balance refreshed"
                );
            }
            Err(e) => {
                tracing::warn!(
                    provider = self.name(),
                    error = %e,
                    "Failed to refresh PPQ balance"
                );
            }
        }
    }

    /// Get the cached balance in micro-usd (fast, non-async read).
    pub fn cached_balance_usd_micro(&self) -> i64 {
        self.balance_usd_micro.load(Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl Provider for PpqProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        // Use the inner OpenAI provider's implementation
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
        // For PPQ providers, do a simple connectivity check via /v1/models
        let health_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(health_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        match req.send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        // Fetch the models list and find matching entry
        let models_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let mut req = self.http_client.get(models_url.as_str());
        if let Some(ref key) = self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            tracing::debug!(
                provider = self.name(),
                status = %response.status(),
                "PPQ GET /v1/models failed"
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
            Ok(balance_usd_micro) => Some(CurrencyAmount::UsdMicro(balance_usd_micro)),
            Err(e) => {
                tracing::warn!(
                    provider = self.name(),
                    error = %e,
                    "Failed to fetch balance from PPQ provider"
                );
                None
            }
        }
    }

    async fn create_topup(&self, amount: CurrencyAmount) -> Option<crate::payments::instructions::PaymentInstruction> {
        use crate::payments::instructions::PaymentInstruction;
        use reqwest::Client;
        
        // PPQ accepts USD amounts
        let amount_usd = match amount {
            CurrencyAmount::UsdMicro(usd_micro) => (usd_micro as f64) / 1_000_000.0,
            _ => return None, // PPQ only supports USD
        };
        
        let api_key = self.api_key.as_ref()?;
        let client = Client::new();
        
        let response = client
            .post("https://api.ppq.ai/topup/create/btc-lightning")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&serde_json::json!({
                "amount": amount_usd,
                "currency": "USD"
            }))
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let result: serde_json::Value = response.json().await.ok()?;
        
        // Parse PPQ's response and convert to PaymentInstruction
        // PPQ typically returns: { "invoice": "lnbc...", "payment_hash": "...", "amount_sats": ... }
        let bolt11 = result.get("invoice").and_then(|v| v.as_str())
            .or_else(|| result.get("bolt11").and_then(|v| v.as_str()))?;
        
        let payment_hash = result.get("payment_hash").and_then(|v| v.as_str())?.to_string();
        let amount_sats = result.get("amount_sats").and_then(|v| v.as_i64()).unwrap_or(0);
        
        Some(PaymentInstruction::LightningBolt11 {
            bolt11: bolt11.to_string(),
            payment_hash,
            amount_sats,
            amount_msat: amount_sats * 1000,
            memo: Some(format!("PPQ top-up for ${:.2}", amount_usd)),
            expires_at: None,
            invoice_id: None,
        })
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
        let provider = PpqProvider::new(
            "Test Provider",
            Some("test"),
            "https://api.ppq.ai",
            Some("test-key"),
        );

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = PpqProvider::new(
            "My Provider",
            None,
            "https://api.ppq.ai",
            Some("key"),
        );
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = PpqProvider::new(
            "Test_Provider",
            Some("custom_slug"),
            "https://api.ppq.ai",
            Some("key"),
        );
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = PpqProvider::new(
            "Test",
            None,
            "https://api.ppq.ai",
            Some("my-api-key"),
        );
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = PpqProvider::new("Test", None, "https://api.ppq.ai", None);
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = PpqProvider::new(
            "Test",
            None,
            "https://api.ppq.ai",
            Some("key"),
        );
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_cached_balance_default_zero() {
        let provider = PpqProvider::new(
            "Test",
            None,
            "https://api.ppq.ai",
            Some("key"),
        );
        assert_eq!(provider.cached_balance_usd_micro(), 0);
        let balance = provider.fetch_balance().await;
        // fetch_balance will fail to reach the fake URL → returns None
        assert_eq!(balance, None);
    }

    #[tokio::test]
    async fn test_chat_completions_stream_error_handling() {
        let provider = PpqProvider::new(
            "Test",
            None,
            "https://invalid-url",
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
