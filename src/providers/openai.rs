use super::*;
use async_openai::config::OpenAIConfig;
use async_openai::Client;
use futures::{stream::BoxStream, StreamExt};

#[derive(Clone)]
pub struct OpenAiProvider {
    name: String,
    slug: String,
    client: Client<OpenAIConfig>,
}

impl OpenAiProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug = slug.unwrap_or(name).to_lowercase().replace(" ", "-").replace("_", "-");
        
        let config = OpenAIConfig::default()
            .with_api_base(base_url)
            .with_api_key(api_key.unwrap_or(""));
        
        Self {
            name: name.to_string(),
            slug,
            client: Client::with_config(config),
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
        let response = self.client.models().list().await?;
        Ok(response.data)
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let response = self.client.chat().create(request.clone()).await?;
        Ok(response)
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>,
        ProviderError,
    > {
        let client = self.client.clone();
        let request = request.clone();

        let stream = async move {
            match client.chat().create_stream(request).await {
                Ok(stream) => {
                    Box::pin(stream.map(|result| {
                        result.map_err(|e| ProviderError::OpenAIError(e))
                    })) as BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>
                }
                Err(e) => {
                    Box::pin(futures::stream::once(async move {
                        Err(ProviderError::OpenAIError(e))
                    })) as BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>
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
        match self.client.models().list().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
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
