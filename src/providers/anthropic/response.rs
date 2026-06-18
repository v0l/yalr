//! Response-side conversions: Anthropic Messages API -> OpenAI chat format,
//! plus error mapping.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::providers::*;

pub(super) fn anthropic_response_to_openai(
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

    let usage = response.usage.as_ref().map(|u| anthropic_usage_to_openai(u));

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

/// Convert an Anthropic usage block to OpenAI `CompletionUsage`.
///
/// Anthropic reports `input_tokens` as the *uncached* prompt remainder, with
/// cache-write (`cache_creation_input_tokens`) and cache-read
/// (`cache_read_input_tokens`) counted separately. OpenAI's `prompt_tokens` is
/// the full prompt size, so we sum all three; the cache-read portion is also
/// surfaced under `prompt_tokens_details.cached_tokens`, matching how OpenAI
/// reports its own automatic prompt caching.
pub(super) fn anthropic_usage_to_openai(u: &async_anthropic::types::Usage) -> CompletionUsage {
    let input = u.input_tokens.unwrap_or(0);
    let cache_creation = u.cache_creation_input_tokens.unwrap_or(0);
    let cache_read = u.cache_read_input_tokens.unwrap_or(0);
    let prompt_tokens = input + cache_creation + cache_read;
    let completion_tokens = u.output_tokens.unwrap_or(0);

    let prompt_tokens_details = if cache_creation > 0 || cache_read > 0 {
        Some(async_openai::types::chat::PromptTokensDetails {
            audio_tokens: None,
            cached_tokens: Some(cache_read),
        })
    } else {
        None
    };

    CompletionUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        completion_tokens_details: None,
        prompt_tokens_details,
    }
}

/// Map Anthropic error to our ProviderError.
pub(super) fn map_anthropic_error(err: async_anthropic::errors::AnthropicError) -> ProviderError {
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

#[cfg(test)]
mod tests {
    use super::*;

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
                        ..Default::default()
                    },
                ),
            ]),
            model: Some("claude-3-5-sonnet-20241022".to_string()),
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: Some(async_anthropic::types::Usage {
                input_tokens: Some(10),
                output_tokens: Some(5),
                ..Default::default()
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
    async fn test_usage_maps_cache_tokens() {
        let usage = async_anthropic::types::Usage {
            input_tokens: Some(100),
            output_tokens: Some(30),
            cache_creation_input_tokens: Some(20),
            cache_read_input_tokens: Some(500),
        };
        let mapped = anthropic_usage_to_openai(&usage);
        // prompt_tokens is the full prompt: uncached + cache-write + cache-read.
        assert_eq!(mapped.prompt_tokens, 620);
        assert_eq!(mapped.completion_tokens, 30);
        assert_eq!(mapped.total_tokens, 650);
        let details = mapped.prompt_tokens_details.expect("cache details present");
        assert_eq!(details.cached_tokens, Some(500));
    }

    #[tokio::test]
    async fn test_usage_no_cache_details_when_uncached() {
        let usage = async_anthropic::types::Usage {
            input_tokens: Some(42),
            output_tokens: Some(7),
            ..Default::default()
        };
        let mapped = anthropic_usage_to_openai(&usage);
        assert_eq!(mapped.prompt_tokens, 42);
        assert!(mapped.prompt_tokens_details.is_none());
    }

    #[tokio::test]
    async fn test_response_to_openai_tool_calls() {
        let response = async_anthropic::types::CreateMessagesResponse {
            id: Some("msg_1".to_string()),
            content: Some(vec![
                async_anthropic::types::MessageContent::ToolUse(async_anthropic::types::ToolUse {
                    id: "call_1".to_string(),
                    name: "get_weather".to_string(),
                    input: serde_json::json!({ "city": "Paris" }),
                    ..Default::default()
                }),
            ]),
            model: Some("claude-3-haiku-20240307".to_string()),
            stop_reason: Some("tool_use".to_string()),
            stop_sequence: None,
            usage: Some(async_anthropic::types::Usage { input_tokens: Some(5), output_tokens: Some(3), ..Default::default() }),
        };
        let result = anthropic_response_to_openai(&response, "claude-3-haiku-20240307");
        assert_eq!(
            result.choices[0].finish_reason,
            Some(async_openai::types::chat::FinishReason::ToolCalls)
        );
        let calls = result.choices[0].message.tool_calls.as_ref().expect("tool calls");
        assert_eq!(calls.len(), 1);
        match &calls[0] {
            async_openai::types::chat::ChatCompletionMessageToolCalls::Function(f) => {
                assert_eq!(f.id, "call_1");
                assert_eq!(f.function.name, "get_weather");
                assert!(f.function.arguments.contains("Paris"));
            }
            _ => panic!("expected function tool call"),
        }
    }
}
