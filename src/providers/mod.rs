pub mod llamacpp;
pub mod openai;
pub mod provider_trait;

pub use llamacpp::LlamaCppProvider;
pub use openai::OpenAiProvider;

// Re-export async-openai types for easy swapping
pub use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
    CreateChatCompletionResponse, CreateChatCompletionStreamResponse,
    ServiceTier, CompletionUsage, FinishReason, ChatChoiceLogprobs, Role,
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
    pub service_tier: Option<ServiceTier>,
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
