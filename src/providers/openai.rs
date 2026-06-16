use super::*;
use async_openai::config::OpenAIConfig;
use async_openai::Client;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use crate::router::{Modality, ModelRuntimeInfo};

/// Error response from API - supports both direct and nested formats via untagged enum
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ErrorResponse {
    /// Direct error format: { "error": { "message": "...", "code": "..." } }
    Direct { error: ErrorDetail },
    /// Nested error format: { "detail": { "error": { "message": "...", "code": "..." } } }
    Nested { detail: NestedDetail },
}

/// Error detail structure - code can be string or int (OpenRouter returns int)
#[derive(Debug, Clone, Deserialize)]
struct ErrorDetail {
    message: String,
    #[serde(default, deserialize_with = "deserialize_optional_string_or_number")]
    code: Option<String>,
    #[serde(default)]
    metadata: Option<ErrorMetadata>,
}

/// Provider-specific error metadata (e.g. OpenRouter rate-limit hints)
#[derive(Debug, Clone, Deserialize)]
struct ErrorMetadata {
    /// Suggested retry delay in seconds (OpenRouter on 429 upstream rate limits)
    #[serde(default)]
    retry_after_seconds: Option<u64>,
}

/// Deserialize a field that can be either string or integer, returning Option<String>
/// Reusable helper for error codes and similar fields that vary between providers
fn deserialize_optional_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde_json::Value;

    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => Ok(Some(n.to_string())),
        Value::Null => Ok(None),
        _ => Err(D::Error::custom("expected string or number")),
    }
}

/// Nested detail wrapper for { "detail": { "error": ... } }
#[derive(Debug, Clone, Deserialize)]
struct NestedDetail {
    error: ErrorDetail,
}

impl ErrorResponse {
    /// Extract error message and code from any format
    fn extract(&self) -> ErrorDetail {
        match self {
            ErrorResponse::Direct { error } => error.clone(),
            ErrorResponse::Nested { detail } => detail.error.clone(),
        }
    }
}

/// Convert an `ErrorDetail` into a richly-typed `ProviderError`.
///
/// Classifies rate limits (429 / `insufficient_quota`), payment errors, and
/// generic server errors so the routing engine can retry / back off correctly.
fn error_detail_to_provider_error(detail: ErrorDetail) -> ProviderError {
    let code_str = detail.code.as_deref();
    let numeric_code = code_str.and_then(|c| c.parse::<u16>().ok());
    let retry_after_ms = detail
        .metadata
        .as_ref()
        .and_then(|m| m.retry_after_seconds)
        .map(|s| s.saturating_mul(1000));

    // Rate limit: numeric 429, or known string codes
    let is_rate_limit = numeric_code == Some(429)
        || matches!(code_str, Some("rate_limit_exceeded") | Some("429"));
    if is_rate_limit {
        return ProviderError::RateLimit {
            // Default to 30s if upstream didn't provide a hint
            retry_after_ms: retry_after_ms.unwrap_or(30_000),
            message: detail.message,
        };
    }

    // Payment / quota errors
    if matches!(code_str, Some("insufficient_quota") | Some("insufficient_balance"))
        || numeric_code == Some(402)
    {
        return ProviderError::ServerError {
            message: detail.message,
            status_code: Some(402),
        };
    }

    ProviderError::ServerError {
        message: detail.message,
        status_code: numeric_code,
    }
}

/// Map an `async_openai` error into a richer `ProviderError`.
///
/// async-openai's `ApiError.code` is typed as `Option<String>`, so when an
/// upstream (e.g. OpenRouter) returns an integer `code` (such as `429`), the
/// library fails to deserialize the error body and surfaces a generic
/// `JSONDeserialize` error — losing the rate-limit classification and any
/// `retry_after_seconds` hint. This helper re-parses the raw body (which the
/// `JSONDeserialize` variant carries) using our own lenient `ErrorResponse`.
fn map_openai_error(err: async_openai::error::OpenAIError) -> ProviderError {
    use async_openai::error::OpenAIError;

    if let OpenAIError::JSONDeserialize(_, ref content) = err {
        if let Ok(parsed) = serde_json::from_str::<ErrorResponse>(content) {
            return error_detail_to_provider_error(parsed.extract());
        }
    }
    ProviderError::OpenAIError(err)
}

#[derive(Clone)]
pub struct OpenAiProvider {
    name: String,
    slug: String,
    pub(crate) client: Client<OpenAIConfig>,
    /// Raw reqwest client for lightweight health checks
    http_client: reqwest::Client,
    /// Base URL (no trailing slash) for constructing health check URLs
    base_url: String,
    /// API key for health check auth (empty string = no auth)
    api_key: String,
    /// Cached model list
    models_cache: Arc<RwLock<Option<(Vec<Model>, Instant)>>>,
}

impl OpenAiProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug = slug.unwrap_or(name).to_lowercase().replace(" ", "-").replace("_", "-");
        
        // Strip trailing slash to avoid double slashes in API URLs
        let base_url = base_url.trim_end_matches('/');
        
        let config = OpenAIConfig::default()
            .with_api_base(base_url)
            .with_api_key(api_key.unwrap_or(""));
        
        Self {
            name: name.to_string(),
            slug,
            client: Client::with_config(config),
            http_client: reqwest::Client::new(),
            base_url: base_url.to_string(),
            api_key: api_key.unwrap_or("").to_string(),
            models_cache: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        // Return cached models if fresh (< 5 min)
        {
            let guard = self.models_cache.read().await;
            if let Some((ref cached, cached_at)) = *guard {
                if cached_at.elapsed() < std::time::Duration::from_secs(300) {
                    return Ok(cached.clone());
                }
            }
        }
        let response = self.client.models().list().await?;
        let models = response.data;
        *self.models_cache.write().await = Some((models.clone(), Instant::now()));
        Ok(models)
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let response = self
            .client
            .chat()
            .create(request.clone())
            .await
            .map_err(map_openai_error)?;
        Ok(response)
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        use crate::providers::StreamingChunk;
        use futures::StreamExt;

        let client = self.client.clone();
        let request = request.clone();
        let provider_name = self.name().to_string();
        let request_model = request.model.clone();

        // Serialize request once at the start
        let request_value = serde_json::to_value(request)
            .map_err(|e| ProviderError::Other(e.into()))?;

        let stream = async move {
            match client.chat().create_stream_byot(request_value.clone()).await {
                Ok(stream) => {
                    Box::pin(stream.map(move |result: Result<serde_json::Value, async_openai::error::OpenAIError>| {
                        match result {
                            Ok(json_value) => {
                                // Try to deserialize as error response first (handles both formats via untagged enum)
                                if let Ok(error_response) = serde_json::from_value::<ErrorResponse>(json_value.clone()) {
                                    let error_detail = error_response.extract();
                                    tracing::warn!(
                                        provider = %provider_name,
                                        error_message = %error_detail.message,
                                        error_code = ?error_detail.code,
                                        "Stream returned error response"
                                    );
                                    return Err(error_detail_to_provider_error(error_detail));
                                }

                                // Not an error, try to deserialize as streaming chunk
                                match serde_json::from_value::<StreamingChunk>(json_value.clone()) {
                                    Ok(chunk) => Ok(chunk),
                                    Err(de_error) => {
                                        // Log the raw JSON that failed to deserialize for debugging
                                        tracing::error!(
                                            provider = %provider_name,
                                            error = %de_error,
                                            raw_json = %json_value,
                                            "Failed to deserialize streaming chunk"
                                        );
                                        Err(ProviderError::Other(
                                            format!("Stream decode error: {} (raw JSON: {})", de_error, json_value).into()
                                        ))
                                    }
                                }
                            }
                            Err(e) => {
                                // Log the OpenAI error with more context
                                tracing::error!(
                                    provider = %provider_name,
                                    error = %e,
                                    "Stream failed"
                                );
                                Err(ProviderError::OpenAIError(e))
                            }
                        }
                    })) as BoxStream<'static, Result<StreamingChunk, ProviderError>>
                }
                Err(e) => {
                    // Log stream setup errors with full context
                    tracing::error!(
                        provider = %provider_name,
                        request_model = %request_model,
                        error = %e,
                        "Stream setup failed"
                    );
                    Box::pin(futures::stream::once(async move {
                        Err(ProviderError::OpenAIError(e))
                    })) as BoxStream<'static, Result<StreamingChunk, ProviderError>>
                }
            }
        };

        Ok(async_stream::stream! {
            let s = stream.await;
            futures::pin_mut!(s);
            while let Some(item) = s.next().await {
                yield item;
            }
        }.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        // Lightweight GET request to /models — just check if server responds.
        // Avoids deserializing the full model list every health check interval.
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut req = self.http_client.get(&url);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        } else {
            tracing::warn!(provider = %self.name(), "No API key configured for OpenAI-compatible provider");
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

    async fn responses(&self, request: &CreateResponse) -> Result<ApiResponse, ProviderError> {
        let response = self
            .client
            .responses()
            .create(request.clone())
            .await
            .map_err(map_openai_error)?;
        Ok(response)
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        match self.client.models().retrieve(model_id).await {
            Ok(model) => {
                let mut additional_fields = HashMap::new();
                additional_fields.insert("object".to_string(), serde_json::json!(model.object));
                additional_fields.insert("created".to_string(), serde_json::json!(model.created));
                additional_fields.insert("owned_by".to_string(), serde_json::json!(model.owned_by));
                
                let runtime_info = ModelRuntimeInfo {
                    model_id: model_id.to_string(),
                    context_length: None,
                    quantization: None,
                    variant: None,
                    parameter_size: None,
                    max_output_tokens: None,
                    max_concurrency: None,
                    modalities: vec![Modality::Text],
                    additional_fields,
                };
                
                Ok(Some(runtime_info))
            }
            Err(_e) => {
                // Some providers (e.g., vLLM) don't support the retrieve endpoint
                // or may not recognize certain model names. Return None gracefully.
                tracing::debug!("Model retrieve not supported or model not found: {}", model_id);
                Ok(None)
            }
        }
    }
}

pub fn convert_message_role(role: &str) -> ChatCompletionRequestMessage {
    match role {
        "system" => ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
            content: ChatCompletionRequestSystemMessageContent::Text(String::new()),
            name: None,
        }),
        "user" => ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Text(String::new()),
            name: None,
        }),
        "assistant" => ChatCompletionRequestMessage::Assistant(
            async_openai::types::chat::ChatCompletionRequestAssistantMessage {
                content: None,
                refusal: None,
                name: None,
                tool_calls: None,
                audio: None,
                ..Default::default()
            },
        ),
        _ => ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Text(String::new()),
            name: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ErrorType;

    #[test]
    fn test_error_response_nested_format() {
        let json_str = r#"{
            "detail": {
                "error": {
                    "message": "Insufficient balance: 232747 mSats required for this model. 202576 available.",
                    "type": "insufficient_quota",
                    "code": "insufficient_balance"
                }
            },
            "request_id": "de069d4f-38a3-4b08-a80b-cdfdda4eae8d"
        }"#;
        
        let error_response: ErrorResponse = serde_json::from_str(json_str).unwrap();
        let detail = error_response.extract();
        
        assert_eq!(detail.message, "Insufficient balance: 232747 mSats required for this model. 202576 available.");
        assert_eq!(detail.code, Some("insufficient_balance".to_string()));
    }

    #[test]
    fn test_error_response_direct_format() {
        let json_str = r#"{
            "error": {
                "message": "Some error message",
                "code": "some_code"
            }
        }"#;
        
        let error_response: ErrorResponse = serde_json::from_str(json_str).unwrap();
        let detail = error_response.extract();
        
        assert_eq!(detail.message, "Some error message");
        assert_eq!(detail.code, Some("some_code".to_string()));
    }

    #[test]
    fn test_error_response_missing_code() {
        let json_str = r#"{
            "error": {
                "message": "Error without code"
            }
        }"#;
        
        let error_response: ErrorResponse = serde_json::from_str(json_str).unwrap();
        let detail = error_response.extract();
        
        assert_eq!(detail.message, "Error without code");
        assert_eq!(detail.code, None);
    }

    #[test]
    fn test_openrouter_integer_code_rate_limit() {
        // OpenRouter returns code as an integer (429), which async-openai cannot
        // parse. Our lenient ErrorResponse must handle it and classify it as a
        // rate limit with the retry_after hint.
        let body = r#"{"error":{"message":"Provider returned error","code":429,"metadata":{"raw":"rate-limited upstream","provider_name":"Darkbloom","is_byok":false,"retry_after_seconds":30,"retry_after_seconds_raw":30}},"user_id":"user_abc"}"#;

        let parsed: ErrorResponse = serde_json::from_str(body).unwrap();
        let detail = parsed.extract();
        assert_eq!(detail.code, Some("429".to_string()));

        let err = error_detail_to_provider_error(detail);
        match err {
            ProviderError::RateLimit { retry_after_ms, message } => {
                assert_eq!(retry_after_ms, 30_000);
                assert_eq!(message, "Provider returned error");
            }
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[test]
    fn test_map_openai_error_reparses_jsondeserialize() {
        let body = r#"{"error":{"message":"Provider returned error","code":429,"metadata":{"retry_after_seconds":15}}}"#;
        // Simulate async-openai's failure: it tried to parse code as a string.
        let serde_err = serde_json::from_str::<String>("429").unwrap_err();
        let oai_err = async_openai::error::OpenAIError::JSONDeserialize(serde_err, body.to_string());

        match map_openai_error(oai_err) {
            ProviderError::RateLimit { retry_after_ms, .. } => {
                assert_eq!(retry_after_ms, 15_000);
            }
            other => panic!("expected RateLimit, got {other:?}"),
        }
    }

    #[test]
    fn test_error_detail_insufficient_balance_maps_402() {
        let detail = ErrorDetail {
            message: "Insufficient balance".to_string(),
            code: Some("insufficient_balance".to_string()),
            metadata: None,
        };
        match error_detail_to_provider_error(detail) {
            ProviderError::ServerError { status_code, .. } => assert_eq!(status_code, Some(402)),
            other => panic!("expected ServerError(402), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = OpenAiProvider::new("Test Provider", Some("test"), "http://localhost:8080", Some("test-key"));

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = OpenAiProvider::new("My Provider", None, "http://localhost:8080", Some("key"));
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = OpenAiProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:8080", Some("key"));
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = OpenAiProvider::new("Test", None, "http://localhost:8080", Some("my-api-key"));
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = OpenAiProvider::new("Test", None, "http://localhost:8080", None);
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = OpenAiProvider::new("Test", None, "http://localhost:8080", Some("key"));
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_list_models_error_handling() {
        let provider = OpenAiProvider::new("Test", None, "http://invalid-url", Some("key"));
        let result = provider.list_models().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_provider_error_error_type() {
        let rate_limit_error = ProviderError::RateLimit {
            retry_after_ms: 1000,
            message: "Rate limited".to_string(),
        };
        assert_eq!(rate_limit_error.error_type(), ErrorType::RateLimit);
        assert_eq!(rate_limit_error.retry_after_ms(), Some(1000));
        assert!(rate_limit_error.is_recoverable());

        let timeout_error = ProviderError::Timeout;
        assert_eq!(timeout_error.error_type(), ErrorType::Timeout);
        assert_eq!(timeout_error.retry_after_ms(), None);
        assert!(timeout_error.is_recoverable());

        let server_error = ProviderError::ServerError {
            message: "Internal error".to_string(),
            status_code: Some(500),
        };
        assert_eq!(server_error.error_type(), ErrorType::ServerError);
        assert_eq!(server_error.status_code(), Some(500));
        assert!(server_error.is_recoverable());

        let auth_error = ProviderError::Authentication("Invalid key".to_string());
        assert_eq!(auth_error.error_type(), ErrorType::Authentication);
        assert!(!auth_error.is_recoverable());

        let not_found_error = ProviderError::NotFound("Model not found".to_string());
        assert_eq!(not_found_error.error_type(), ErrorType::NotFound);
        assert!(!not_found_error.is_recoverable());
    }

    #[tokio::test]
    async fn test_provider_error_clone() {
        let error = ProviderError::RateLimit {
            retry_after_ms: 2000,
            message: "Too many requests".to_string(),
        };
        let cloned = error.clone();
        assert_eq!(error.retry_after_ms(), cloned.retry_after_ms());
    }

#[tokio::test]
    async fn test_chat_completions_stream_error_handling() {
        let provider = OpenAiProvider::new("Test", None, "http://invalid-url", Some("key"));
        let request = CreateChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            ..Default::default()
        };
        let result = provider.chat_completions_stream(&request);
        assert!(result.is_ok());
    }
}
