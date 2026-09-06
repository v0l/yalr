use super::*;
use async_openai::types::chat::{CreateChatCompletionRequest, CreateChatCompletionResponse};
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// PPQ top-up request body
#[derive(Debug, Serialize)]
struct PpqTopupRequest {
    amount: f64,
    currency: String,
}

/// PPQ top-up invoice response structure
#[derive(Debug, Deserialize)]
struct PpqTopupResponse {
    /// Invoice ID for status tracking
    invoice_id: String,
    /// Lightning invoice string (BOLT11)
    #[serde(alias = "lightning_invoice")]
    #[serde(alias = "invoice")]
    #[serde(alias = "bolt11")]
    invoice: String,
    /// Amount in USD
    amount: f64,
    /// Currency (e.g., "USD")
    currency: String,
    /// Unix timestamp for expiration
    expires_at: i64,
    /// Optional: checkout URL for web payment
    #[serde(default)]
    checkout_url: Option<String>,
    /// Optional: crypto amount in BTC
    #[serde(default)]
    crypto_amount_due: Option<f64>,
    /// Optional: creation timestamp
    #[serde(default)]
    created_at: Option<i64>,
}

/// PPQ credit balance response structure
#[derive(Debug, Deserialize)]
struct PpqBalanceResponse {
    /// Balance in USD (e.g. 10.169783715)
    balance: f64,
}

/// PPQ credit balance request body
#[derive(Debug, Serialize)]
struct PpqBalanceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    credit_id: Option<String>,
}

/// PpqProvider — wraps the PPQ.ai OpenAI-compatible API and adds balance tracking.
///
/// PPQ.ai (PayPerQ) is an OpenAI-compatible API provider that accepts Lightning
/// payments and tracks credit balances. This provider connects to the PPQ API
/// and polls the credit balance endpoint so the admin dashboard can show remaining
/// credit.
///
/// API Documentation: https://ppq.ai/llms.txt
/// Top-up Endpoints:
/// - POST https://api.ppq.ai/topup/create/btc-lightning — Bitcoin Lightning (USD, BTC, SATS)
/// - POST https://api.ppq.ai/topup/create/btc — Bitcoin on-chain
/// - POST https://api.ppq.ai/topup/create/ltc — Litecoin
/// - POST https://api.ppq.ai/topup/create/lbtc — Liquid Bitcoin
/// - POST https://api.ppq.ai/topup/create/xmr — Monero
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
        let body = PpqBalanceRequest {
            credit_id: self.credit_id.clone(),
        };
        req = req.json(&body);

        let resp = req.send().await.map_err(|e| ProviderError::Other(e.into()))?;

        if !resp.status().is_success() {
            return Err(ProviderError::ServerError {
                message: format!("Balance endpoint returned {}", resp.status()),
                status_code: Some(resp.status().as_u16()),
            });
        }

        let balance_resp: PpqBalanceResponse =
            resp.json().await.map_err(|e| ProviderError::Other(e.into()))?;

        // PPQ returns balance in USD (e.g. 10.169783715), convert to micro-usd
        let balance_usd_micro = (balance_resp.balance * 1_000_000.0) as i64;

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
            _ => {
                tracing::warn!(provider = self.name(), "PPQ only supports USD top-ups, got {:?}", amount);
                return None;
            }
        };
        
        let api_key = match &self.api_key {
            Some(key) => key,
            None => {
                tracing::error!(provider = self.name(), "PPQ top-up requires API key but none is configured");
                return None;
            }
        };
        
        let client = Client::new();
        
        let response = match client
            .post("https://api.ppq.ai/topup/create/btc-lightning")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&PpqTopupRequest {
                amount: amount_usd,
                currency: "USD".to_string(),
            })
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::error!(provider = self.name(), error = %e, "Failed to send PPQ top-up request");
                return None;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(provider = self.name(), status = %status, error_body = %error_body, "PPQ top-up endpoint returned error");
            return None;
        }

        let ppq_response: PpqTopupResponse = match response.json().await {
            Ok(response) => response,
            Err(e) => {
                tracing::error!(provider = self.name(), error = %e, "Failed to parse PPQ top-up response as struct");
                return None;
            }
        };
        
        // Calculate sats from BTC amount if available, otherwise estimate from USD
        let amount_sats = ppq_response
            .crypto_amount_due
            .map(|btc| (btc * 100_000_000.0) as i64)
            .unwrap_or(0);
        
        // PPQ doesn't return payment_hash, so we use the invoice_id
        let payment_hash = ppq_response.invoice_id.clone();
        
        Some(PaymentInstruction::LightningBolt11 {
            bolt11: ppq_response.invoice,
            payment_hash,
            amount_sats,
            amount_msat: amount_sats * 1000,
            memo: Some(format!("PPQ top-up for ${:.2}", amount_usd)),
            expires_at: Some(ppq_response.expires_at),
            invoice_id: ppq_response
                .invoice_id
                .parse::<i64>()
                .ok(), // Try to parse as i64, but PPQ uses string IDs so this will likely be None
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
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/credits/balance"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let provider = PpqProvider::new("Test", None, &mock_server.uri(), Some("key"));
        assert_eq!(provider.cached_balance_usd_micro(), 0);
        assert_eq!(provider.fetch_balance().await, None);
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

    #[tokio::test]
    async fn test_ppq_topup_response_deserialization() {
        // Test with the actual PPQ response format
        let json_response = r#"{
            "created_at": 1779368687,
            "invoice_id": "JX2uuRERXWLhTkGpcPzgpV",
            "amount": 10,
            "currency": "USD",
            "expires_at": 1779369587,
            "checkout_url": "https://btcpay0.voltageapp.io/i/JX2uuRERXWLhTkGpcPzgpV",
            "lightning_invoice": "lnbc129450n1p4q7qh0pp5glfxyjtl2mmuj3mvcw2gynkr6jm3m74qhaxxj9dxhxzye8k6g",
            "crypto_amount_due": 0.00012945
        }"#;

        let response: PpqTopupResponse = serde_json::from_str(json_response).unwrap();
        
        assert_eq!(response.invoice_id, "JX2uuRERXWLhTkGpcPzgpV");
        assert_eq!(response.amount, 10.0);
        assert_eq!(response.currency, "USD");
        assert_eq!(response.expires_at, 1779369587);
        assert_eq!(response.checkout_url, Some("https://btcpay0.voltageapp.io/i/JX2uuRERXWLhTkGpcPzgpV".to_string()));
        assert_eq!(response.crypto_amount_due, Some(0.00012945));
        assert!(response.created_at.is_some());
        assert!(response.invoice.starts_with("lnbc"));
    }

    #[tokio::test]
    async fn test_ppq_topup_response_with_invoice_alias() {
        // Test with "invoice" field instead of "lightning_invoice"
        let json_response = r#"{
            "invoice_id": "test123",
            "amount": 5.5,
            "currency": "USD",
            "expires_at": 1234567890,
            "invoice": "lnbc50n1p234567890",
            "crypto_amount_due": 0.00005000
        }"#;

        let response: PpqTopupResponse = serde_json::from_str(json_response).unwrap();
        
        assert_eq!(response.invoice_id, "test123");
        assert_eq!(response.amount, 5.5);
        assert_eq!(response.invoice, "lnbc50n1p234567890");
        assert_eq!(response.crypto_amount_due, Some(0.00005000));
    }

    #[tokio::test]
    async fn test_ppq_topup_response_with_bolt11_alias() {
        // Test with "bolt11" field
        let json_response = r#"{
            "invoice_id": "abc789",
            "amount": 20,
            "currency": "USD",
            "expires_at": 9876543210,
            "bolt11": "lnbc200n1p987654",
            "crypto_amount_due": 0.00020000
        }"#;

        let response: PpqTopupResponse = serde_json::from_str(json_response).unwrap();
        
        assert_eq!(response.invoice_id, "abc789");
        assert_eq!(response.invoice, "lnbc200n1p987654");
        assert_eq!(response.amount, 20.0);
    }

    #[tokio::test]
    async fn test_ppq_topup_response_minimal() {
        // Test with minimal required fields only
        let json_response = r#"{
            "invoice_id": "minimal123",
            "amount": 1.0,
            "currency": "USD",
            "expires_at": 1111111111,
            "invoice": "lnbc100n1p"
        }"#;

        let response: PpqTopupResponse = serde_json::from_str(json_response).unwrap();
        
        assert_eq!(response.invoice_id, "minimal123");
        assert_eq!(response.amount, 1.0);
        assert_eq!(response.currency, "USD");
        assert_eq!(response.expires_at, 1111111111);
        assert!(response.checkout_url.is_none());
        assert!(response.crypto_amount_due.is_none());
        assert!(response.created_at.is_none());
    }

    #[tokio::test]
    async fn test_create_topup_without_api_key_returns_none() {
        let provider = PpqProvider::new(
            "Test PPQ",
            Some("ppq-test"),
            "https://api.ppq.ai",
            None, // No API key
        );

        let amount = CurrencyAmount::UsdMicro(10_000_000); // $10.00
        let result = provider.create_topup(amount).await;
        
        assert!(result.is_none(), "create_topup should return None when API key is missing");
    }

    #[tokio::test]
    async fn test_create_topup_non_usd_currency_returns_none() {
        let provider = PpqProvider::new(
            "Test PPQ",
            Some("ppq-test"),
            "https://api.ppq.ai",
            Some("test-api-key"),
        );

        let amount = CurrencyAmount::Sats(1000); // Non-USD currency
        let result = provider.create_topup(amount).await;
        
        assert!(result.is_none(), "create_topup should return None for non-USD currency");
    }

    #[tokio::test]
    async fn test_ppq_balance_response_deserialization() {
        // Test the actual PPQ balance response format
        let json_response = r#"{"balance": 10.169783715}"#;
        let response: PpqBalanceResponse = serde_json::from_str(json_response).unwrap();
        assert_eq!(response.balance, 10.169783715);

        // Convert to micro-usd (what the provider stores)
        let balance_usd_micro = (response.balance * 1_000_000.0) as i64;
        assert_eq!(balance_usd_micro, 10_169_783);
    }

    #[tokio::test]
    async fn test_ppq_balance_response_zero_balance() {
        let json_response = r#"{"balance": 0.0}"#;
        let response: PpqBalanceResponse = serde_json::from_str(json_response).unwrap();
        assert_eq!(response.balance, 0.0);
        let balance_usd_micro = (response.balance * 1_000_000.0) as i64;
        assert_eq!(balance_usd_micro, 0);
    }

    #[tokio::test]
    async fn test_ppq_balance_response_large_balance() {
        // Test a large balance ($999.99)
        let json_response = r#"{"balance": 999.99}"#;
        let response: PpqBalanceResponse = serde_json::from_str(json_response).unwrap();
        let balance_usd_micro = (response.balance * 1_000_000.0) as i64;
        assert_eq!(balance_usd_micro, 999_990_000);
    }

    #[tokio::test]
    async fn test_ppq_balance_response_invalid_json() {
        // Missing required field should fail
        let json_response = r#"{}"#;
        let result = serde_json::from_str::<PpqBalanceResponse>(json_response);
        assert!(result.is_err());

        // Wrong field type should fail
        let json_response = r#"{"balance": "not-a-number"}"#;
        let result = serde_json::from_str::<PpqBalanceResponse>(json_response);
        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_sats_from_btc() {
        // Test BTC to sats conversion
        let btc: f64 = 0.00012945;
        let sats = (btc * 100_000_000.0) as i64;
        
        assert_eq!(sats, 12945);
    }
}
