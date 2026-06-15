//! Wire types and translation between OpenAI chat format and the Anthropic
//! Messages API for the OAuth provider.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::providers::*;

/// The Claude Code prefix must be the first system block for OAuth tokens.
pub const CLAUDE_CODE_SYSTEM: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

#[derive(Debug, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    pub messages: Vec<AnthropicMessage>,
    pub system: Vec<SystemBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    pub stream: bool,
}

#[derive(Debug, Deserialize)]
pub struct MessagesResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub block_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub message: Option<StreamMessage>,
    #[serde(default)]
    pub delta: Option<StreamDelta>,
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub struct StreamMessage {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StreamDelta {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
}

/// Convert OpenAI-format messages into (system blocks, anthropic messages).
pub fn convert(
    messages: &[ChatCompletionRequestMessage],
) -> (Vec<SystemBlock>, Vec<AnthropicMessage>) {
    let mut system_text = String::new();
    let mut out: Vec<AnthropicMessage> = Vec::new();

    let push_system = |text: &str, buf: &mut String| {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(text);
    };

    for msg in messages {
        match msg {
            ChatCompletionRequestMessage::System(m) => match &m.content {
                ChatCompletionRequestSystemMessageContent::Text(t) => {
                    push_system(t, &mut system_text)
                }
                ChatCompletionRequestSystemMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(t) = p;
                        push_system(&t.text, &mut system_text);
                    }
                }
            },
            ChatCompletionRequestMessage::Developer(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(t) => {
                    push_system(t, &mut system_text)
                }
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(t) = p;
                        push_system(&t.text, &mut system_text);
                    }
                }
            },
            ChatCompletionRequestMessage::User(m) => {
                let content = match &m.content {
                    ChatCompletionRequestUserMessageContent::Text(t) => t.clone(),
                    ChatCompletionRequestUserMessageContent::Array(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            async_openai::types::chat::ChatCompletionRequestUserMessageContentPart::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                out.push(AnthropicMessage { role: "user", content });
            }
            ChatCompletionRequestMessage::Assistant(m) => {
                let content = match &m.content {
                    Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(t)) => t.clone(),
                    Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Array(parts)) => parts
                        .iter()
                        .filter_map(|p| match p {
                            async_openai::types::chat::ChatCompletionRequestAssistantMessageContentPart::Text(t) => Some(t.text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                    None => String::new(),
                };
                out.push(AnthropicMessage { role: "assistant", content });
            }
            ChatCompletionRequestMessage::Tool(m) => {
                let content = match &m.content {
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => t.clone(),
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            async_openai::types::chat::ChatCompletionRequestToolMessageContentPart::Text(t) => Some(t.text.clone()),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                out.push(AnthropicMessage {
                    role: "user",
                    content: format!("[Tool result: {}]", content),
                });
            }
            ChatCompletionRequestMessage::Function(_) => {}
        }
    }

    let mut system = vec![SystemBlock {
        block_type: "text",
        text: CLAUDE_CODE_SYSTEM.to_string(),
    }];
    if !system_text.is_empty() {
        system.push(SystemBlock {
            block_type: "text",
            text: system_text,
        });
    }
    (system, out)
}

pub fn build_request(request: &CreateChatCompletionRequest, stream: bool) -> MessagesRequest {
    let (system, messages) = convert(&request.messages);
    let stop_sequences = request.stop.as_ref().map(|s| match s {
        async_openai::types::chat::StopConfiguration::String(v) => vec![v.clone()],
        async_openai::types::chat::StopConfiguration::StringArray(v) => v.clone(),
    });
    MessagesRequest {
        model: request.model.clone(),
        max_tokens: request.max_completion_tokens.unwrap_or(4096),
        messages,
        system,
        temperature: request.temperature,
        stop_sequences,
        stream,
    }
}

pub fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

pub fn finish_reason(stop: Option<&str>) -> Option<FinishReason> {
    match stop {
        Some("end_turn") | Some("max_tokens") | Some("stop_sequence") => Some(FinishReason::Stop),
        Some("tool_use") => Some(FinishReason::ToolCalls),
        _ => None,
    }
}

pub fn map_status_error(status: u16, body: String) -> ProviderError {
    match status {
        401 | 403 => ProviderError::Authentication(body),
        404 => ProviderError::NotFound(body),
        429 => ProviderError::RateLimit {
            retry_after_ms: 2000,
            message: body,
        },
        _ => ProviderError::ServerError {
            message: body,
            status_code: Some(status),
        },
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
        extra_fields: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_injects_claude_code_system_first() {
        let messages = vec![
            ChatCompletionRequestMessage::System(
                async_openai::types::chat::ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text("Be terse".into()),
                    name: None,
                },
            ),
            ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hi".into()),
                    name: None,
                },
            ),
        ];
        let (system, msgs) = convert(&messages);
        assert_eq!(system[0].text, CLAUDE_CODE_SYSTEM);
        assert_eq!(system[1].text, "Be terse");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_reason(Some("end_turn")), Some(FinishReason::Stop));
        assert_eq!(finish_reason(Some("tool_use")), Some(FinishReason::ToolCalls));
        assert_eq!(finish_reason(None), None);
    }

    #[test]
    fn build_request_sets_stream_and_max_tokens() {
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hi".into()),
                    name: None,
                },
            )],
            ..Default::default()
        };
        let body = build_request(&req, true);
        assert!(body.stream);
        assert_eq!(body.max_tokens, 4096);
        assert_eq!(body.system[0].text, CLAUDE_CODE_SYSTEM);
    }
}
