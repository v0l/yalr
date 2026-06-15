//! Wire types and translation between OpenAI chat-completions format and the
//! ChatGPT/Codex Responses API used by the OpenAI OAuth provider.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::providers::*;

#[derive(Debug, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub part_type: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct InputItem {
    #[serde(rename = "type")]
    pub item_type: &'static str,
    pub role: &'static str,
    pub content: Vec<ContentPart>,
}

#[derive(Debug, Serialize)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub input: Vec<InputItem>,
    pub stream: bool,
    pub store: bool,
}

/// Convert OpenAI chat messages into (instructions, input items).
/// System/developer messages are concatenated into `instructions`.
pub fn convert(
    messages: &[ChatCompletionRequestMessage],
) -> (Option<String>, Vec<InputItem>) {
    let mut instructions = String::new();
    let mut input: Vec<InputItem> = Vec::new();

    let push_instr = |text: &str, buf: &mut String| {
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(text);
    };

    for msg in messages {
        match msg {
            ChatCompletionRequestMessage::System(m) => match &m.content {
                ChatCompletionRequestSystemMessageContent::Text(t) => push_instr(t, &mut instructions),
                ChatCompletionRequestSystemMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(t) = p;
                        push_instr(&t.text, &mut instructions);
                    }
                }
            },
            ChatCompletionRequestMessage::Developer(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(t) => push_instr(t, &mut instructions),
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(t) = p;
                        push_instr(&t.text, &mut instructions);
                    }
                }
            },
            ChatCompletionRequestMessage::User(m) => {
                let text = match &m.content {
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
                input.push(message_item("user", "input_text", text));
            }
            ChatCompletionRequestMessage::Assistant(m) => {
                let text = match &m.content {
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
                input.push(message_item("assistant", "output_text", text));
            }
            ChatCompletionRequestMessage::Tool(m) => {
                let text = match &m.content {
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => t.clone(),
                    async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            async_openai::types::chat::ChatCompletionRequestToolMessageContentPart::Text(t) => Some(t.text.clone()),
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                input.push(message_item("user", "input_text", format!("[Tool result: {}]", text)));
            }
            ChatCompletionRequestMessage::Function(_) => {}
        }
    }

    let instructions = if instructions.is_empty() {
        None
    } else {
        Some(instructions)
    };
    (instructions, input)
}

fn message_item(role: &'static str, part_type: &'static str, text: String) -> InputItem {
    InputItem {
        item_type: "message",
        role,
        content: vec![ContentPart { part_type, text }],
    }
}

pub fn build_request(request: &CreateChatCompletionRequest, stream: bool) -> ResponsesRequest {
    let (instructions, input) = convert(&request.messages);
    ResponsesRequest {
        model: request.model.clone(),
        instructions,
        input,
        stream,
        store: false,
    }
}

// --- SSE event parsing ---

#[derive(Debug, Deserialize)]
pub struct StreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub delta: Option<String>,
    #[serde(default)]
    pub response: Option<ResponseObject>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseObject {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub usage: Option<ResponseUsage>,
    #[serde(default)]
    pub output: Vec<OutputItem>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ResponseUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
}

#[derive(Debug, Deserialize)]
pub struct OutputItem {
    #[serde(default)]
    pub content: Vec<OutputContent>,
}

#[derive(Debug, Deserialize)]
pub struct OutputContent {
    #[serde(default)]
    pub text: Option<String>,
}

impl ResponseObject {
    /// Concatenate all output text blocks.
    pub fn output_text(&self) -> String {
        self.output
            .iter()
            .flat_map(|o| o.content.iter())
            .filter_map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join("")
    }
}

pub fn now_secs() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

pub fn usage_to_openai(u: &ResponseUsage) -> CompletionUsage {
    CompletionUsage {
        prompt_tokens: u.input_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: u.input_tokens + u.output_tokens,
        completion_tokens_details: None,
        prompt_tokens_details: None,
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

pub fn final_chunk(id: &str, model: &str, usage: Option<CompletionUsage>) -> StreamingChunk {
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
            finish_reason: Some(FinishReason::Stop),
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
    fn convert_splits_instructions_and_input() {
        let messages = vec![
            ChatCompletionRequestMessage::System(
                async_openai::types::chat::ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text("sys".into()),
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
        let (instr, input) = convert(&messages);
        assert_eq!(instr.as_deref(), Some("sys"));
        assert_eq!(input.len(), 1);
        assert_eq!(input[0].role, "user");
        assert_eq!(input[0].content[0].part_type, "input_text");
    }

    #[test]
    fn build_request_disables_store() {
        let req = CreateChatCompletionRequest {
            model: "gpt-5".into(),
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
        assert!(!body.store);
    }

    #[test]
    fn response_output_text_concats() {
        let r = ResponseObject {
            id: Some("r1".into()),
            usage: None,
            output: vec![OutputItem {
                content: vec![
                    OutputContent { text: Some("a".into()) },
                    OutputContent { text: Some("b".into()) },
                ],
            }],
        };
        assert_eq!(r.output_text(), "ab");
    }
}
