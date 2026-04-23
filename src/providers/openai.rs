use super::*;
use async_openai::config::OpenAIConfig;
use async_openai::Client;
use futures::{stream::BoxStream, StreamExt};
use std::collections::HashMap;
use crate::router::ModelRuntimeInfo;

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
        BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        use crate::providers::StreamingChunk;
        use futures::StreamExt;

        let client = self.client.clone();
        let request = request.clone();

        // Serialize request once at the start
        let request_value = serde_json::to_value(request)
            .map_err(|e| ProviderError::ProviderError(format!("Failed to serialize request: {}", e)))?;

        let stream = async move {
            match client.chat().create_stream_byot(request_value).await {
                Ok(stream) => {
                    Box::pin(stream.map(|result| {
                        result
                            .map_err(|e| ProviderError::OpenAIError(e))
                            .and_then(|json_value: serde_json::Value| {
                                // Deserialize the raw JSON value to our custom type
                                // This preserves all fields including reasoning_content
                                serde_json::from_value(json_value)
                                    .map_err(|e| ProviderError::ProviderError(format!("Failed to deserialize chunk: {}", e)))
                            })
                    })) as BoxStream<'static, Result<StreamingChunk, ProviderError>>
                }
                Err(e) => {
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
        match self.client.models().list().await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        match self.client.models().retrieve(model_id).await {
            Ok(model) => {
                let mut additional_fields = HashMap::new();
                additional_fields.insert("object".to_string(), serde_json::json!(model.object));
                additional_fields.insert("created".to_string(), serde_json::json!(model.created));
                additional_fields.insert("owned_by".to_string(), serde_json::json!(model.owned_by));
                
                Ok(Some(ModelRuntimeInfo::from_api_response(model_id, additional_fields)))
            }
            Err(e) => Err(ProviderError::OpenAIError(e)),
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
