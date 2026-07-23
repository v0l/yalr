//! Streaming conversion: Anthropic SSE events -> OpenAI chat completion chunks.

use std::time::{SystemTime, UNIX_EPOCH};

use futures::stream::BoxStream;
use futures::StreamExt;

use crate::providers::*;
use super::convert::{build_anthropic_request, convert_messages};
use super::response::map_anthropic_error;

/// Build a streaming chat completion as an OpenAI-compatible chunk stream.
pub(super) fn chat_completions_stream(
    client: async_anthropic::Client,
    request: &CreateChatCompletionRequest,
) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
    let (system, messages) = convert_messages(&request.messages);
    let mut anthropic_request = build_anthropic_request(system, messages, request);
    anthropic_request.stream = true;

    let model_name = request.model.clone();

        let stream = async_stream::stream! {
            let mut anthropic_stream = client.messages().create_stream(anthropic_request).await;

            let mut message_id = String::new();
            // Anthropic reports prompt + cache token counts on `message_start`
            // and the output count on `message_delta`. Hold the start usage so
            // the final usage chunk can report both together.
            let mut start_usage: Option<async_anthropic::types::Usage> = None;
            // Maps an Anthropic content-block index to its OpenAI tool_call index.
            let mut tool_block_indices: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
            let mut next_tool_index: u32 = 0;

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
                        start_usage = message.usage;
                    }
                    async_anthropic::types::MessagesStreamEvent::ContentBlockStart { index, content_block } => {
                        if let async_anthropic::types::MessageContent::ToolUse(tool_use) = content_block {
                            let tool_index = next_tool_index;
                            next_tool_index += 1;
                            tool_block_indices.insert(index, tool_index);

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
                                        tool_calls: Some(vec![ChatCompletionMessageToolCallChunk {
                                            index: tool_index,
                                            id: Some(tool_use.id.clone()),
                                            r#type: Some(async_openai::types::chat::FunctionType::Function),
                                            function: Some(async_openai::types::chat::FunctionCallStream {
                                                name: Some(tool_use.name.clone()),
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
                            };
                            yield Ok(chunk);
                        }
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
                            async_anthropic::types::ContentBlockDelta::InputJsonDelta { partial_json } => {
                                let tool_index = tool_block_indices.get(&index).copied().unwrap_or(0);
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
                                };
                                yield Ok(chunk);
                            }
                        }
                    }
                    async_anthropic::types::MessagesStreamEvent::MessageDelta { delta, usage } => {
                        let finish_reason = match delta.stop_reason.as_deref() {
                            Some("end_turn") | Some("max_tokens") | Some("stop_sequence") => {
                                Some("stop".to_string())
                            }
                            Some("tool_use") => Some("tool_calls".to_string()),
                            _ => None,
                        };

                        // Combine the prompt/cache counts seen on `message_start`
                        // with the output count on this `message_delta`, then map
                        // through the shared usage converter so cache tokens land
                        // in `prompt_tokens_details.cached_tokens`.
                        let mut cache_write_tokens = 0u32;
                        let usage_info = usage.map(|delta_usage| {
                            let mut merged = start_usage.clone().unwrap_or_default();
                            merged.output_tokens = delta_usage.output_tokens;
                            if delta_usage.input_tokens.is_some() {
                                merged.input_tokens = delta_usage.input_tokens;
                            }
                            cache_write_tokens = merged.cache_creation_input_tokens.unwrap_or(0);
                            super::response::anthropic_usage_to_openai(&merged)
                        });

                        let created = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as u32;

                        // Carry the cache-write count out-of-band; OpenAI usage has
                        // no field for it. The router consumes and strips this.
                        let mut extra_fields = std::collections::HashMap::new();
                        if cache_write_tokens > 0 {
                            extra_fields.insert(
                                CACHE_WRITE_TOKENS_FIELD.to_string(),
                                serde_json::json!(cache_write_tokens),
                            );
                        }

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
                            extra_fields,
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
