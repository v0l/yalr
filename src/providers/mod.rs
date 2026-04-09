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
};
pub use async_openai::types::models::Model;

pub use provider_trait::*;
