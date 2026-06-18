//! Anthropic response/stream -> OpenAI chat output conversion.

use super::*;
use crate::providers::*;

/// Split response content blocks into joined text and OpenAI tool calls.
pub fn split_content_blocks(
    blocks: &[ContentBlock],
) -> (
    String,
    Option<Vec<async_openai::types::chat::ChatCompletionMessageToolCalls>>,
) {
    let text = blocks
        .iter()
        .filter_map(|b| b.text.clone())
        .collect::<Vec<_>>()
        .join("");

    let calls: Vec<_> = blocks
        .iter()
        .filter(|b| b.block_type.as_deref() == Some("tool_use"))
        .filter_map(|b| {
            let id = b.id.clone()?;
            let name = b.name.clone()?;
            let arguments = b
                .input
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default();
            Some(async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                async_openai::types::chat::ChatCompletionMessageToolCall {
                    id,
                    function: async_openai::types::chat::FunctionCall { name, arguments },
                },
            ))
        })
        .collect();

    let tool_calls = if calls.is_empty() { None } else { Some(calls) };
    (text, tool_calls)
}

/// Build a streaming chunk that opens a tool call (id + name, empty args).
pub fn tool_call_start_chunk(
    id: &str,
    model: &str,
    tool_index: u32,
    call_id: String,
    name: String,
) -> StreamingChunk {
    StreamingChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: now_secs(),
        model: model.to_string(),
        choices: vec![StreamingChoice {
            index: 0,
            delta: StreamingDelta {
                content: None,
                role: None,
                refusal: None,
                tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
                    index: tool_index,
                    id: Some(call_id),
                    r#type: Some(async_openai::types::chat::FunctionType::Function),
                    function: Some(async_openai::types::chat::FunctionCallStream {
                        name: Some(name),
                        arguments: Some(String::new()),
                    }),
                }]),
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
    }
}

/// Build a streaming chunk that appends partial JSON args to a tool call.
pub fn tool_call_args_chunk(
    id: &str,
    model: &str,
    tool_index: u32,
    partial_json: String,
) -> StreamingChunk {
    StreamingChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: now_secs(),
        model: model.to_string(),
        choices: vec![StreamingChoice {
            index: 0,
            delta: StreamingDelta {
                content: None,
                role: None,
                refusal: None,
                tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
                    index: tool_index,
                    id: None,
                    r#type: None,
                    function: Some(async_openai::types::chat::FunctionCallStream {
                        name: None,
                        arguments: Some(partial_json),
                    }),
                }]),
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
    }
}

pub fn text_chunk(id: &str, model: &str, text: String) -> StreamingChunk {
    StreamingChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: now_secs(),
        model: model.to_string(),
        choices: vec![StreamingChoice {
            index: 0,
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
    }
}

pub fn final_chunk(
    id: &str,
    model: &str,
    finish_reason: Option<FinishReason>,
    usage: Option<CompletionUsage>,
    cache_write_tokens: u32,
) -> StreamingChunk {
    // Carry the cache-write count out-of-band; OpenAI usage has no field for it.
    // The router consumes and strips this before the chunk reaches the client.
    let mut extra_fields = std::collections::HashMap::new();
    if cache_write_tokens > 0 {
        extra_fields.insert(
            CACHE_WRITE_TOKENS_FIELD.to_string(),
            serde_json::json!(cache_write_tokens),
        );
    }
    StreamingChunk {
        id: id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created: now_secs(),
        model: model.to_string(),
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
        usage,
        extra_fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_content_blocks_extracts_tool_calls() {
        let blocks = vec![
            ContentBlock {
                block_type: Some("text".into()),
                text: Some("hi".into()),
                id: None,
                name: None,
                input: None,
            },
            ContentBlock {
                block_type: Some("tool_use".into()),
                text: None,
                id: Some("call_1".into()),
                name: Some("get_weather".into()),
                input: Some(serde_json::json!({ "city": "Paris" })),
            },
        ];
        let (text, calls) = split_content_blocks(&blocks);
        assert_eq!(text, "hi");
        let calls = calls.expect("tool calls");
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
