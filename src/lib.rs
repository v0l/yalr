pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod metrics;
pub mod providers;
pub mod router;
pub mod state;

pub use metrics::{HealthConfig, HealthState, ProviderHealthState};

// Re-export types from providers module for centralized type management
pub use providers::{
    CreateChatCompletionRequest as ChatCompletionRequest,
    CreateChatCompletionResponse as ChatCompletionResponse,
    StreamingChunk, StreamingChoice, StreamingDelta,
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
