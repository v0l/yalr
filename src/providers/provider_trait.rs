use async_openai::error::OpenAIError;
use async_openai::types::chat::{
    CreateChatCompletionRequest, CreateChatCompletionResponse,
    CreateChatCompletionStreamResponse,
};
use async_openai::types::models::Model;
use async_trait::async_trait;
use futures::stream::BoxStream;

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
        BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>,
        ProviderError,
    >;

    async fn health_check(&self) -> Result<bool, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("OpenAI error: {0}")]
    OpenAIError(#[from] OpenAIError),

    #[error("Provider error: {0}")]
    ProviderError(String),
}
