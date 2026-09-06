use serde::{Deserialize, Serialize};

use super::*;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// Routstr balance info response
#[derive(Debug, Deserialize)]
struct RoutstrBalanceResponse {
    /// Balance in millisatoshis
    #[serde(alias = "balance")]
    balance_msat: i64,
}

/// Routstr invoice creation request body
#[derive(Debug, Serialize)]
struct RoutstrInvoiceRequest<'a> {
    amount_sats: i64,
    purpose: &'a str,
    api_key: &'a str,
}

/// Routstr invoice response
#[derive(Debug, Deserialize)]
struct RoutstrInvoiceResponse {
    bolt11: String,
    #[serde(default)]
    payment_hash: Option<String>,
    #[serde(default)]
    amount_sats: Option<i64>,
    #[serde(default)]
    expires_at: Option<i64>,
}

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
    models_cache: ModelsCache,
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
            models_cache: ModelsCache::new(),
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

        let body: RoutstrBalanceResponse =
            resp.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        Ok(body.balance_msat)
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
        if let Some(cached) = self.models_cache.get().await {
            return Ok(cached);
        }

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

        let body: ModelListResponse =
            response.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        // Parse models flexibly - extract just the fields we need
        let result: Vec<Model> = body.data.into_iter().map(|item| Model {
            id: item.id,
            object: item.object.unwrap_or_else(|| "model".to_string()),
            created: item.created.unwrap_or(0),
            owned_by: item.owned_by.unwrap_or_else(|| "routstr".to_string()),
        }).collect();

        self.models_cache.store(result.clone()).await;
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

        let body: ModelListResponse =
            response.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        let model_entry = body
            .data
            .iter()
            .find(|m| m.id == model_id);

        let mut additional_fields = std::collections::HashMap::new();
        if let Some(entry) = model_entry {
            additional_fields = entry.extra.clone();
            additional_fields.remove("id");
        }

        let context_length = model_entry.and_then(|m| m.context_length);

        let max_concurrency = model_entry.and_then(|m| m.max_concurrency);

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

    async fn create_topup(&self, amount: CurrencyAmount) -> Option<crate::payments::instructions::PaymentInstruction> {
        use crate::payments::instructions::PaymentInstruction;
        use reqwest::Client;
        
        // Routstr only accepts sats
        let amount_sats = match amount {
            CurrencyAmount::Sats(sats) => sats,
            CurrencyAmount::Msats(msats) => msats / 1000,
            _ => {
                tracing::warn!(provider = self.name(), "Routstr only supports Lightning top-ups in sats");
                return None;
            }
        };
        
        let api_key = self.api_key.as_ref()?;
        let client = Client::new();
        
        // Note: This implementation uses /v1/balance/lightning/invoice (not RIP-08 /lightning/invoice)
        // and requires api_key in request body (not Authorization header).
        // RIP-08 spec: https://github.com/Routstr/protocol/blob/main/RIP-08.md
        let invoice_url = self.base_url.join("v1/balance/lightning/invoice").ok()?;
        
        // Per RIP-08 spec, only amount_sats and purpose are required.
        // This specific Routstr implementation also requires api_key in the body.
        let request_body = RoutstrInvoiceRequest {
            amount_sats,
            purpose: "topup",
            api_key,
        };
        
        tracing::info!(
            provider = self.name(),
            amount_sats,
            invoice_url = invoice_url.as_str(),
            "Creating Routstr top-up invoice"
        );
        
        let response = match client
            .post(invoice_url.as_str())
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!(
                    provider = self.name(),
                    error = %e,
                    "Failed to create Routstr top-up invoice"
                );
                return None;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                provider = self.name(),
                status = %status,
                error_body = %error_body,
                "Routstr invoice creation failed"
            );
            return None;
        }

        let result: RoutstrInvoiceResponse = match response.json().await {
            Ok(json) => json,
            Err(e) => {
                tracing::error!(
                    provider = self.name(),
                    error = %e,
                    "Failed to parse Routstr invoice response"
                );
                return None;
            }
        };
        
        // Parse Routstr's response and convert to PaymentInstruction
        let bolt11 = result.bolt11.clone();
        
        let payment_hash = match result.payment_hash {
            Some(hash) => hash,
            None => {
                tracing::error!(
                    provider = self.name(),
                    response = ?result,
                    "Routstr did not return payment_hash in invoice response"
                );
                return None;
            }
        };
        
        let expires_at = result.expires_at;
        
        Some(PaymentInstruction::LightningBolt11 {
            bolt11: bolt11.to_string(),
            payment_hash,
            amount_sats,
            amount_msat: amount_sats * 1000,
            memo: Some(format!("Routstr top-up for {}", self.name())),
            expires_at,
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
