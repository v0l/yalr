use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    CreateChatCompletionRequest, CreateChatCompletionResponse,
};
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use async_openai::types::models::Model;
use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::metrics::ErrorType;
use crate::router::ModelRuntimeInfo;
use crate::providers::StreamingChunk;

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;

    fn slug(&self) -> &str;

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError>;

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError>;

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<StreamingChunk, ProviderError>>,
        ProviderError,
    >;

    async fn health_check(&self) -> Result<bool, ProviderError>;

    /// Get the Responses API for this provider.
    /// Returns None if the provider does not support the Responses API.
    async fn responses(&self, request: &CreateResponse) -> Result<ApiResponse, ProviderError> {
        let _ = request;
        Err(ProviderError::Other(
            "This provider does not support the Responses API".to_string().into()
        ))
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        let _ = model_id;
        Ok(None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("OpenAI error: {0}")]
    OpenAIError(#[from] OpenAIError),

    #[error("Rate limit exceeded. Retry after: {retry_after_ms}ms")]
    RateLimit {
        retry_after_ms: u64,
        message: String,
    },

    #[error("Server error: {message} (status: {status_code:?})")]
    ServerError {
        message: String,
        status_code: Option<u16>,
    },

    #[error("Request timeout")]
    Timeout,

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl Clone for ProviderError {
    fn clone(&self) -> Self {
        match self {
            ProviderError::OpenAIError(_) => {
                ProviderError::Other("OpenAI error".to_string().into())
            }
            ProviderError::RateLimit { retry_after_ms, message } => {
                ProviderError::RateLimit {
                    retry_after_ms: *retry_after_ms,
                    message: message.clone(),
                }
            }
            ProviderError::ServerError { message, status_code } => ProviderError::ServerError {
                message: message.clone(),
                status_code: *status_code,
            },
            ProviderError::Timeout => ProviderError::Timeout,
            ProviderError::Authentication(msg) => ProviderError::Authentication(msg.clone()),
            ProviderError::NotFound(msg) => ProviderError::NotFound(msg.clone()),
            ProviderError::Other(_) => ProviderError::Other("Error".to_string().into()),
        }
    }
}

impl ProviderError {
    pub fn error_type(&self) -> ErrorType {
        match self {
            ProviderError::RateLimit { .. } => ErrorType::RateLimit,
            ProviderError::Timeout => ErrorType::Timeout,
            ProviderError::Authentication(_) => ErrorType::Authentication,
            ProviderError::NotFound(_) => ErrorType::NotFound,
            ProviderError::ServerError { .. } => ErrorType::ServerError,
            _ => ErrorType::Other,
        }
    }

    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            ProviderError::RateLimit { retry_after_ms, .. } => Some(*retry_after_ms),
            _ => None,
        }
    }

    pub fn status_code(&self) -> Option<u16> {
        match self {
            ProviderError::ServerError { status_code, .. } => *status_code,
            _ => None,
        }
    }

    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimit { .. }
                | ProviderError::Timeout
                | ProviderError::ServerError { .. }
        )
    }

    /// Returns true if this error is transient and may succeed on retry.
    /// Transient errors include timeouts, rate limits, server errors (5xx),
    /// and OpenAI errors which often represent network issues or 502/503s.
    pub fn is_transient(&self) -> bool {
        match self {
            ProviderError::RateLimit { .. } => true,
            ProviderError::Timeout => true,
            ProviderError::ServerError { status_code, .. } => {
                // 5xx errors are transient
                status_code.map_or(true, |code| code >= 500)
            }
            ProviderError::OpenAIError(_) => {
                // OpenAI errors from async_openai often represent
                // network issues, 502 Bad Gateway, 503 Service Unavailable, etc.
                true
            }
            ProviderError::Authentication(_) => false,
            ProviderError::NotFound(_) => false,
            ProviderError::Other(_) => true,
        }
    }
}
