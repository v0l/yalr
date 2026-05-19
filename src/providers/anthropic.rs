//! Anthropic provider implementation using the async-anthropic crate.
//!
//! This provider translates between OpenAI-compatible chat completion types
//! (used by the router) and Anthropic's native Messages API format.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use super::*;

/// Anthropic-specific provider using the async-anthropic crate.
///
/// Anthropic uses the Messages API (not OpenAI-compatible chat completions),
/// so this provider translates between the two formats internally.
#[derive(Clone)]
pub struct AnthropicProvider {
    name: String,
    slug: String,
    client: async_anthropic::Client,
    /// Raw reqwest client for lightweight health checks
    http_client: reqwest::Client,
    /// Base URL for constructing health check URLs
    base_url: String,
    /// API key for health check auth header
    api_key: String,
    /// Cached model list
    models_cache: Arc<RwLock<Option<(Vec<Model>, Instant)>>>,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider.
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug = slug
            .unwrap_or(name)
            .to_lowercase()
            .replace(' ', "-")
            .replace('_', "-");

        let client = build_client(base_url, api_key);

        Self {
            name: name.to_string(),
            slug,
            client,
            http_client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.unwrap_or("").to_string(),
            models_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Convert OpenAI chat messages to Anthropic native format.
    /// Returns (optional system prompt, message list).
    fn convert_messages(
        &self,
        messages: &[ChatCompletionRequestMessage],
    ) -> (Option<String>, Vec<async_anthropic::types::Message>) {
        let mut system = String::new();
        let mut anthropic_messages: Vec<async_anthropic::types::Message> = Vec::new();

        for msg in messages {
            match msg {
                ChatCompletionRequestMessage::System(sys_msg) => {
                    match &sys_msg.content {
                        ChatCompletionRequestSystemMessageContent::Text(text) => {
                            if !system.is_empty() {
                                system.push('\n');
                            }
                            system.push_str(text);
                        }
                        ChatCompletionRequestSystemMessageContent::Array(parts) => {
                            for part in parts {
                                match part {
                                    async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(text_part) => {
                                        if !system.is_empty() {
                                            system.push('\n');
                                        }
                                        system.push_str(&text_part.text);
                                    }
                                }
                            }
                        }
                    }
                }
                ChatCompletionRequestMessage::User(user_msg) => {
                    let content = match &user_msg.content {
                        ChatCompletionRequestUserMessageContent::Text(text) => text.clone(),
                        ChatCompletionRequestUserMessageContent::Array(parts) => {
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    async_openai::types::chat::ChatCompletionRequestUserMessageContentPart::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    };
                    let message = async_anthropic::types::Message {
                        role: async_anthropic::types::MessageRole::User,
                        content: async_anthropic::types::MessageContentList(vec![
                            async_anthropic::types::MessageContent::Text(
                                async_anthropic::types::Text { text: content },
                            ),
                        ]),
                    };
                    anthropic_messages.push(message);
                }
                ChatCompletionRequestMessage::Assistant(assistant_msg) => {
                    let text = match &assistant_msg.content {
                        Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(t)) => t.clone(),
                        Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Array(parts)) => {
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    async_openai::types::chat::ChatCompletionRequestAssistantMessageContentPart::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        }
                        None => String::new(),
                    };

                    let content_list = async_anthropic::types::MessageContentList(vec![
                        async_anthropic::types::MessageContent::Text(
                            async_anthropic::types::Text { text },
                        ),
                    ]);

                    let message = async_anthropic::types::Message {
                        role: async_anthropic::types::MessageRole::Assistant,
                        content: content_list,
                    };
                    anthropic_messages.push(message);
                }
                ChatCompletionRequestMessage::Tool(tool_msg) => {
                    let content = match &tool_msg.content {
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => t.clone(),
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(parts) => {
                            parts.iter().filter_map(|p| match p {
                                async_openai::types::chat::ChatCompletionRequestToolMessageContentPart::Text(t) => Some(t.text.clone()),
                            }).collect::<Vec<_>>().join("\n")
                        }
                    };
                    let message = async_anthropic::types::Message {
                        role: async_anthropic::types::MessageRole::User,
                        content: async_anthropic::types::MessageContentList(vec![
                            async_anthropic::types::MessageContent::Text(
                                async_anthropic::types::Text {
                                    text: format!("[Tool result: {}]", content),
                                },
                            ),
                        ]),
                    };
                    anthropic_messages.push(message);
                }
                ChatCompletionRequestMessage::Developer(dev_msg) => {
                    match &dev_msg.content {
                        async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(text) => {
                            if !system.is_empty() {
                                system.push('\n');
                            }
                            system.push_str(text);
                        }
                        async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                            for part in parts {
                                match part {
                                    async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(text_part) => {
                                        if !system.is_empty() {
                                            system.push('\n');
                                        }
                                        system.push_str(&text_part.text);
                                    }
                                }
                            }
                        }
                    }
                }
                ChatCompletionRequestMessage::Function(_) => {
                    tracing::debug!("Skipping unsupported function message type");
                }
            }
        }

        let system = if system.is_empty() {
            None
        } else {
            Some(system)
        };

        (system, anthropic_messages)
    }
}

/// Build the async-anthropic client with the given base URL and API key.
fn build_client(
    base_url: &str,
    api_key: Option<&str>,
) -> async_anthropic::Client {
    // Simplify: build client chaining setters inline
    let mut builder = async_anthropic::Client::builder();
    if let Some(key) = api_key {
        if !key.is_empty() {
            builder.api_key(key.to_string());
        }
    }
    if !base_url.is_empty() {
        builder.base_url(base_url.to_string());
    }
    builder.build().unwrap_or_default()
}

/// Build an Anthropic create messages request from the OpenAI-format request.
fn build_anthropic_request(
    system: Option<String>,
    messages: Vec<async_anthropic::types::Message>,
    request: &CreateChatCompletionRequest,
) -> async_anthropic::types::CreateMessagesRequest {
    let mut builder = async_anthropic::types::CreateMessagesRequestBuilder::default();
    builder.model(&request.model);
    builder.messages(messages);
    builder.max_tokens(request.max_completion_tokens.unwrap_or(4096) as i32);

    if let Some(temp) = request.temperature {
        builder.temperature(temp);
    }

    if let Some(ref stop) = request.stop {
        match stop {
            async_openai::types::chat::StopConfiguration::String(s) => {
                builder.stop_sequences(vec![s.clone()]);
            }
            async_openai::types::chat::StopConfiguration::StringArray(arr) => {
                builder.stop_sequences(arr.clone());
            }
        }
    }

    if let Some(ref metadata) = request.metadata {
        // Convert Metadata struct to serde_json Map
        let map = serde_json::to_value(metadata)
            .ok()
            .and_then(|v| v.as_object().cloned());
        if let Some(m) = map {
            builder.metadata(m);
        }
    }

    if let Some(ref system_content) = system {
        builder.system(system_content.clone());
    }

    builder.build().expect("Failed to build Anthropic messages request")
}

/// Convert an Anthropic Messages response to OpenAI-compatible format.
fn anthropic_response_to_openai(
    response: &async_anthropic::types::CreateMessagesResponse,
    model_name: &str,
) -> CreateChatCompletionResponse {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32;

    let id = response
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let content_blocks = response.content.clone().unwrap_or_default();

    // Build text from content blocks
    let text = content_blocks
        .iter()
        .filter_map(|block| match block {
            async_anthropic::types::MessageContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");

    // Build tool calls from content blocks
    let tool_calls: Option<Vec<async_openai::types::chat::ChatCompletionMessageToolCalls>> = {
        let calls: Vec<_> = content_blocks
            .iter()
            .filter_map(|block| match block {
                async_anthropic::types::MessageContent::ToolUse(tool_use) => {
                    Some(async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                        async_openai::types::chat::ChatCompletionMessageToolCall {
                            id: tool_use.id.clone(),
                            function: async_openai::types::chat::FunctionCall {
                                name: tool_use.name.clone(),
                                arguments: serde_json::to_string(&tool_use.input)
                                    .unwrap_or_default(),
                            },
                        },
                    ))
                }
                _ => None,
            })
            .collect();
        if calls.is_empty() {
            None
        } else {
            Some(calls)
        }
    };

    let finish_reason = match response.stop_reason.as_deref() {
        Some("end_turn") | Some("max_tokens") => {
            Some(async_openai::types::chat::FinishReason::Stop)
        }
        Some("stop_sequence") => Some(async_openai::types::chat::FinishReason::Stop),
        Some("tool_use") => Some(async_openai::types::chat::FinishReason::ToolCalls),
        _ => None,
    };

    let usage = response.usage.as_ref().map(|u| CompletionUsage {
        prompt_tokens: u.input_tokens.unwrap_or(0),
        completion_tokens: u.output_tokens.unwrap_or(0),
        total_tokens: u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0),
        completion_tokens_details: None,
        prompt_tokens_details: None,
    });

    let message = async_openai::types::chat::ChatCompletionResponseMessage {
        content: Some(text),
        refusal: None,
        tool_calls,
        annotations: None,
        role: Role::Assistant,
        function_call: None,
        audio: None,
    };

    let choice = async_openai::types::chat::ChatChoice {
        index: 0,
        message,
        finish_reason,
        logprobs: None,
    };

    CreateChatCompletionResponse {
        id,
        created,
        model: model_name.to_string(),
        choices: vec![choice],
        usage,
        service_tier: None,
        #[allow(deprecated)]
        system_fingerprint: None,
        object: "chat.completion".to_string(),
    }
}

/// Map Anthropic error to our ProviderError.
fn map_anthropic_error(err: async_anthropic::errors::AnthropicError) -> ProviderError {
    match err {
        async_anthropic::errors::AnthropicError::Unauthorized => {
            ProviderError::Authentication("Invalid Anthropic API key".to_string())
        }
        async_anthropic::errors::AnthropicError::BadRequest(msg) => {
            if msg.contains("rate") || msg.contains("Rate") {
                ProviderError::RateLimit {
                    retry_after_ms: 2000,
                    message: msg,
                }
            } else {
                ProviderError::ServerError {
                    message: msg,
                    status_code: Some(400),
                }
            }
        }
        async_anthropic::errors::AnthropicError::ApiError(msg) => {
            if msg.contains("overloaded") {
                ProviderError::RateLimit {
                    retry_after_ms: 5000,
                    message: msg,
                }
            } else if msg.contains("not_found_error") || msg.contains("model not found") {
                ProviderError::NotFound(msg)
            } else {
                ProviderError::ServerError {
                    message: msg,
                    status_code: Some(529),
                }
            }
        }
        async_anthropic::errors::AnthropicError::NetworkError(e) => {
            if e.is_timeout() {
                ProviderError::Timeout
            } else {
                ProviderError::Other(e.into())
            }
        }
        async_anthropic::errors::AnthropicError::DeserializationError(e) => {
            ProviderError::Other(e.into())
        }
        async_anthropic::errors::AnthropicError::StreamError(stream_err) => {
            ProviderError::ServerError {
                message: stream_err.to_string(),
                status_code: None,
            }
        }
        _ => ProviderError::Other(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(),
        ))),
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        // Return cached models if fresh (< 5 min)
        if let Some((ref cached, cached_at)) = *self.models_cache.read().await {
            if cached_at.elapsed() < std::time::Duration::from_secs(300) {
                return Ok(cached.clone());
            }
        }
        let response = self
            .client
            .models()
            .list()
            .await
            .map_err(map_anthropic_error)?;

        let models: Vec<Model> = response
            .data
            .into_iter()
            .map(|m| Model {
                id: m.id,
                object: "model".to_string(),
                created: 0,
                owned_by: m.display_name,
            })
            .collect();

        *self.models_cache.write().await = Some((models.clone(), Instant::now()));
        Ok(models)
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let (system, messages) = self.convert_messages(&request.messages);
        let anthropic_request = build_anthropic_request(system, messages, request);

        let response = self
            .client
            .messages()
            .create(anthropic_request)
            .await
            .map_err(map_anthropic_error)?;

        Ok(anthropic_response_to_openai(&response, &request.model))
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        let (system, messages) = self.convert_messages(&request.messages);
        let mut anthropic_request = build_anthropic_request(system, messages, request);
        anthropic_request.stream = true;

        let client = self.client.clone();
        let model_name = request.model.clone();

        let stream = async_stream::stream! {
            let mut anthropic_stream = client.messages().create_stream(anthropic_request).await;

            let mut message_id = String::new();

            while let Some(result) = anthropic_stream.next().await {
                let event = match result {
                    Ok(event) => event,
                    Err(e) => {
                        yield Err(map_anthropic_error(e));
                        return;
                    }
                };

                match event {
                    async_anthropic::types::MessagesStreamEvent::MessageStart { message, .. } => {
                        message_id = message.id;
                    }
                    async_anthropic::types::MessagesStreamEvent::ContentBlockDelta { index, delta } => {
                        match delta {
                            async_anthropic::types::ContentBlockDelta::TextDelta { text } => {
                                let created = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs() as u32;

                                let chunk = StreamingChunk {
                                    id: message_id.clone(),
                                    object: "chat.completion.chunk".to_string(),
                                    created,
                                    model: model_name.clone(),
                                    choices: vec![StreamingChoice {
                                        index: index as u32,
                                        delta: StreamingDelta {
                                            content: Some(text),
                                            role: None,
                                            refusal: None,
                                            tool_calls: None,
                                            reasoning_content: None,
                                            extra_fields: Default::default(),
                                        },
                                        finish_reason: None,
                                        logprobs: None,
                                    }],
                                    service_tier: None,
                                    #[allow(deprecated)]
                                    system_fingerprint: None,
                                    usage: None,
                                    extra_fields: Default::default(),
                                };
                                yield Ok(chunk);
                            }
                            async_anthropic::types::ContentBlockDelta::InputJsonDelta { .. } => {
                                // Tool call JSON streaming - not surfaced in OpenAI format
                            }
                        }
                    }
                    async_anthropic::types::MessagesStreamEvent::MessageDelta { delta, usage } => {
                        let finish_reason = match delta.stop_reason.as_deref() {
                            Some("end_turn") | Some("max_tokens") => {
                                Some(async_openai::types::chat::FinishReason::Stop)
                            }
                            Some("stop_sequence") => Some(async_openai::types::chat::FinishReason::Stop),
                            Some("tool_use") => Some(async_openai::types::chat::FinishReason::ToolCalls),
                            _ => None,
                        };

                        let usage_info = usage.map(|u| CompletionUsage {
                            prompt_tokens: u.input_tokens.unwrap_or(0),
                            completion_tokens: u.output_tokens.unwrap_or(0),
                            total_tokens: u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0),
                            completion_tokens_details: None,
                            prompt_tokens_details: None,
                        });

                        let created = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as u32;

                        let chunk = StreamingChunk {
                            id: message_id.clone(),
                            object: "chat.completion.chunk".to_string(),
                            created,
                            model: model_name.clone(),
                            choices: vec![StreamingChoice {
                                index: 0,
                                delta: StreamingDelta {
                                    content: None,
                                    role: None,
                                    refusal: None,
                                    tool_calls: None,
                                    reasoning_content: None,
                                    extra_fields: Default::default(),
                                },
                                finish_reason,
                                logprobs: None,
                            }],
                            service_tier: None,
                            #[allow(deprecated)]
                            system_fingerprint: None,
                            usage: usage_info,
                            extra_fields: Default::default(),
                        };
                        yield Ok(chunk);
                    }
                    async_anthropic::types::MessagesStreamEvent::MessageStop => {
                        // Stream complete
                    }
                    _ => {}
                }
            }
        };

        Ok(stream.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        // Lightweight GET to /v1/models — just check if the server responds.
        let url = format!("{}/v1/models?limit=1", self.base_url);
        let mut req = self.http_client.get(&url).header("anthropic-version", "2023-06-01");
        if !self.api_key.is_empty() {
            req = req.header("x-api-key", &self.api_key);
        }
        match req.send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            Some("anthropic"),
            "https://api.anthropic.com",
            Some("test-key"),
        );

        assert_eq!(provider.name(), "Anthropic");
        assert_eq!(provider.slug(), "anthropic");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider = AnthropicProvider::new(
            "My Provider",
            None,
            "https://api.anthropic.com",
            Some("key"),
        );
        assert_eq!(provider.slug(), "my-provider");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            None,
            "https://api.anthropic.com",
            None,
        );
        assert_eq!(provider.name(), "Anthropic");
    }

    #[tokio::test]
    async fn test_convert_messages_simple_user() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            None,
            "https://api.anthropic.com",
            Some("key"),
        );

        let messages = vec![
            ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(
                        "Hello world".to_string(),
                    ),
                    name: None,
                },
            ),
        ];

        let (system, anthropic_msgs) = provider.convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(anthropic_msgs.len(), 1);
        assert_eq!(anthropic_msgs[0].role, async_anthropic::types::MessageRole::User);
        if let Some(async_anthropic::types::MessageContent::Text(t)) = anthropic_msgs[0].content.first() {
            assert_eq!(t.text, "Hello world");
        } else {
            panic!("Expected text content");
        }
    }

    #[tokio::test]
    async fn test_convert_messages_with_system() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            None,
            "https://api.anthropic.com",
            Some("key"),
        );

        let messages = vec![
            ChatCompletionRequestMessage::System(
                async_openai::types::chat::ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text(
                        "You are helpful".to_string(),
                    ),
                    name: None,
                },
            ),
            ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(
                        "Hi".to_string(),
                    ),
                    name: None,
                },
            ),
        ];

        let (system, anthropic_msgs) = provider.convert_messages(&messages);
        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(anthropic_msgs.len(), 1);
    }

    #[tokio::test]
    async fn test_map_anthropic_error_unauthorized() {
        let err = async_anthropic::errors::AnthropicError::Unauthorized;
        let provider_err = map_anthropic_error(err);
        assert!(matches!(provider_err, ProviderError::Authentication(_)));
        assert!(!provider_err.is_recoverable());
    }

    #[tokio::test]
    async fn test_map_anthropic_error_stream_error() {
        let stream_err = async_anthropic::errors::StreamError {
            error_type: "overloaded_error".to_string(),
            message: "Overloaded".to_string(),
        };
        let err = async_anthropic::errors::AnthropicError::StreamError(stream_err);
        let provider_err = map_anthropic_error(err);
        assert!(matches!(provider_err, ProviderError::ServerError { .. }));
        assert!(provider_err.is_recoverable());
    }

    #[tokio::test]
    async fn test_anthropic_response_to_openai() {
        let response = async_anthropic::types::CreateMessagesResponse {
            id: Some("msg_123".to_string()),
            content: Some(vec![
                async_anthropic::types::MessageContent::Text(
                    async_anthropic::types::Text {
                        text: "Hello there!".to_string(),
                    },
                ),
            ]),
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: Some(async_anthropic::types::Usage {
                input_tokens: Some(10),
                output_tokens: Some(5),
            }),
        };

        let result = anthropic_response_to_openai(&response, "claude-3-5-sonnet-20241022");
        assert_eq!(result.id, "msg_123");
        assert_eq!(result.model, "claude-3-5-sonnet-20241022");
        assert_eq!(result.choices.len(), 1);
        assert_eq!(result.choices[0].finish_reason, Some(async_openai::types::chat::FinishReason::Stop));
        assert!(result.usage.is_some());
        let usage = result.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[tokio::test]
    async fn test_chat_completions_invalid_url() {
        let provider = AnthropicProvider::new(
            "Anthropic",
            None,
            "https://invalid.anthropic.example.com",
            Some("test-key"),
        );

        let request = CreateChatCompletionRequest {
            model: "claude-3-haiku-20240307".to_string(),
            messages: vec![
                ChatCompletionRequestMessage::User(
                    async_openai::types::chat::ChatCompletionRequestUserMessage {
                        content: ChatCompletionRequestUserMessageContent::Text(
                            "Hello".to_string(),
                        ),
                        name: None,
                    },
                ),
            ],
            ..Default::default()
        };

        let result = provider.chat_completions(&request).await;
        assert!(result.is_err());
    }
}
