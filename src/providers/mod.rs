/// Standard model entry in a /v1/models response (OpenAI-compatible).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelListEntry {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u32>,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub max_concurrency: Option<u32>,
    /// Catch-all for provider-specific extra fields on model entries
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Standard /v1/models response wrapper (OpenAI-compatible).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelListResponse {
    #[serde(default)]
    pub data: Vec<ModelListEntry>,
}

pub mod llamacpp;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod anthropic;
pub mod anthropic_oauth;
pub mod openai_oauth;
pub mod provider_trait;
pub mod routstr;
pub mod vllm;
pub mod ppq;

pub use llamacpp::LlamaCppProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;
pub use openrouter::OpenRouterProvider;
pub use anthropic::AnthropicProvider;
pub use anthropic_oauth::AnthropicOAuthProvider;
pub use openai_oauth::OpenAiOAuthProvider;
pub use routstr::RoutstrProvider;
pub use vllm::VllmProvider;
pub use ppq::PpqProvider;

use crate::db::{Database, Provider as ProviderRecord, ProviderType};
use std::sync::Arc;

/// Factory function to create a provider instance based on type.
///
/// For OAuth provider types this does NOT have access to the stored tokens and
/// will panic; callers handling OAuth providers must use
/// [`create_provider_from_record`] instead.
pub fn create_provider(
    name: &str,
    slug: Option<&str>,
    base_url: &str,
    api_key: Option<&str>,
    provider_type: ProviderType,
) -> Arc<dyn Provider> {
    match provider_type {
        ProviderType::OpenAi => Arc::new(OpenAiProvider::new(name, slug, base_url, api_key)),
        ProviderType::LlamaCpp => Arc::new(LlamaCppProvider::new(name, slug, base_url, api_key).unwrap()),
        ProviderType::Vllm => Arc::new(VllmProvider::new(name, slug, base_url, api_key)),
        ProviderType::Ollama => Arc::new(OllamaProvider::new(name, slug, base_url, api_key).unwrap()),
        ProviderType::Routstr => Arc::new(RoutstrProvider::new(name, slug, base_url, api_key)),
        ProviderType::OpenRouter => Arc::new(OpenRouterProvider::new(name, slug, base_url, api_key)),
        ProviderType::Anthropic => Arc::new(AnthropicProvider::new(name, slug, base_url, api_key)),
        ProviderType::Ppq => Arc::new(PpqProvider::new(name, slug, base_url, api_key)),
        ProviderType::AnthropicOauth | ProviderType::OpenAiOauth => {
            panic!("OAuth providers must be created via create_provider_from_record")
        }
    }
}

/// Factory that builds a provider from a full DB record, giving OAuth providers
/// access to their stored tokens and a database handle for token refresh.
pub fn create_provider_from_record(
    record: &ProviderRecord,
    db: Arc<Database>,
) -> Arc<dyn Provider> {
    match record.provider_type {
        ProviderType::AnthropicOauth => Arc::new(AnthropicOAuthProvider::new(record, db)),
        ProviderType::OpenAiOauth => Arc::new(OpenAiOAuthProvider::new(record, db)),
        other => create_provider(
            &record.name,
            Some(&record.slug),
            &record.base_url,
            record.api_key.as_deref(),
            other,
        ),
    }
}

/// Auto-detect provider type by trying each implementation's get_runtime_info
/// Returns the detected provider type or None if detection fails
pub async fn detect_provider_type(base_url: &str) -> Option<ProviderType> {
    // Try LlamaCpp first (more specific implementation)
    let llamacpp_provider = LlamaCppProvider::new("detect", None, base_url, None).ok()?;
    if let Ok(Some(_info)) = llamacpp_provider.get_runtime_info("dummy-model").await {
        return Some(ProviderType::LlamaCpp);
    }
    
    // If LlamaCpp fails, assume OpenAI (most compatible)
    Some(ProviderType::OpenAi)
}

// Re-export async-openai types for easy swapping
pub use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
    CreateChatCompletionResponse, CreateChatCompletionStreamResponse,
    CompletionUsage, FinishReason, ChatChoiceLogprobs, Role,
    ChatCompletionMessageToolCallChunk,
};
pub use async_openai::types::models::Model;

pub use provider_trait::*;

/// Custom stream response type that preserves additional fields like reasoning_content
/// from models that support thinking/reasoning content (e.g., Qwen3.5).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingChunk {
    pub id: String,
    pub object: String,
    pub created: u32,
    pub model: String,
    pub choices: Vec<StreamingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    /// Additional fields that may be present in the response (e.g., reasoning_content)
    #[serde(flatten, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingChoice {
    pub index: u32,
    pub delta: StreamingDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<FinishReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<ChatChoiceLogprobs>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>,
    /// Thinking/reasoning content from models that support it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Additional fields that may be present in the delta
    #[serde(flatten, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}
