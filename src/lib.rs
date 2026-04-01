pub mod api;
pub mod config;
pub mod db;
pub mod metrics;
pub mod providers;
pub mod router;

// Re-export types from providers module for centralized type management
pub use providers::{
    CreateChatCompletionRequest as ChatCompletionRequest,
    CreateChatCompletionResponse as ChatCompletionResponse,
    CreateChatCompletionStreamResponse as ChatCompletionChunk,
    ChatCompletionRequestMessage, ChatCompletionRequestMessage as Message,
    ChatCompletionRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent,
    Model,
};
pub use router::engine::{Router, RouterError};
pub use providers::provider_trait::{Provider, ProviderError};
pub use router::strategies::RoutingStrategy;
