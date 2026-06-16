//! Wire types and translation between OpenAI chat format and the Anthropic
//! Messages API for the OAuth provider.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::providers::*;

/// The Claude Code prefix must be the first system block for OAuth tokens.
pub const CLAUDE_CODE_SYSTEM: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

// ---------------------------------------------------------------------------
// Claude Code billing header.
//
// Anthropic now classifies OAuth (Claude Pro/Max subscription) requests that
// don't look like Claude Code as "third-party app usage" and rejects them with
// a 400 disguised as "You're out of extra usage.". To be counted/accepted like
// Claude Code, we prepend a synthetic `x-anthropic-billing-header` system text
// block, mirroring what the official CLI sends. Ported from
// https://github.com/gotgenes/pi-anthropic-auth (src/request-shaping.ts).
//
// CLAUDE_CODE_VERSION must be kept roughly in sync with the current Claude Code
// release. There is no upstream source to import it from; check `claude
// --version` or https://github.com/anthropics/claude-code. If it drifts too far
// from what Anthropic expects, OAuth requests may be rejected.
// ---------------------------------------------------------------------------

/// Claude Code version string embedded in the billing header.
pub const CLAUDE_CODE_VERSION: &str = "2.1.150";
/// Salt used in the billing header suffix hash.
pub const BILLING_HEADER_SALT: &str = "59cf53e54c78";
/// Entrypoint identifier included in the billing header.
pub const CLAUDE_CODE_ENTRYPOINT: &str = "sdk-cli";
/// Character positions sampled from the first user message for the billing hash.
pub const BILLING_HEADER_POSITIONS: [usize; 3] = [4, 7, 20];
/// Marker used to detect (and avoid duplicating) an existing billing block.
pub const BILLING_HEADER_MARKER: &str = "x-anthropic-billing-header:";

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// Build the synthetic Claude Code billing header value from the first user
/// message text. Returns `None` when there is no user text to sample.
pub fn billing_header_value(first_user_text: &str) -> Option<String> {
    if first_user_text.is_empty() {
        return None;
    }

    let chars: Vec<char> = first_user_text.chars().collect();
    let cch: String = sha256_hex(first_user_text).chars().take(5).collect();
    let sampled: String = BILLING_HEADER_POSITIONS
        .iter()
        .map(|&i| chars.get(i).copied().unwrap_or('0'))
        .collect();
    let suffix: String =
        sha256_hex(&format!("{}{}{}", BILLING_HEADER_SALT, sampled, CLAUDE_CODE_VERSION))
            .chars()
            .take(3)
            .collect();

    Some(format!(
        "{} cc_version={}.{}; cc_entrypoint={}; cch={};",
        BILLING_HEADER_MARKER, CLAUDE_CODE_VERSION, suffix, CLAUDE_CODE_ENTRYPOINT, cch
    ))
}

/// Extract the first user message text from an OpenAI-format request, used to
/// derive the billing header (mirrors the official CLI's sampling input).
pub fn first_user_text(messages: &[ChatCompletionRequestMessage]) -> Option<String> {
    for msg in messages {
        if let ChatCompletionRequestMessage::User(m) = msg {
            let text = match &m.content {
                ChatCompletionRequestUserMessageContent::Text(t) => t.clone(),
                ChatCompletionRequestUserMessageContent::Array(parts) => parts
                    .iter()
                    .find_map(|p| match p {
                        async_openai::types::chat::ChatCompletionRequestUserMessageContentPart::Text(t) => {
                            Some(t.text.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default(),
            };
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

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
    let (mut system, messages) = convert(&request.messages);

    // Prepend the Claude Code billing header block (de-duplicated) so the
    // request is accepted/counted like Claude Code rather than rejected as
    // third-party OAuth usage. It must come before the identity block.
    let already_present = system.iter().any(|b| b.text.contains(BILLING_HEADER_MARKER));
    if !already_present {
        if let Some(text) = first_user_text(&request.messages).and_then(|t| billing_header_value(&t)) {
            system.insert(
                0,
                SystemBlock {
                    block_type: "text",
                    text,
                },
            );
        }
    }

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

    fn billing_value(text: &str) -> String {
        billing_header_value(text).unwrap()
    }

    #[test]
    fn billing_header_format_is_stable() {
        let v = billing_value("Please summarize this repository status.");
        assert!(v.starts_with("x-anthropic-billing-header: cc_version=2.1.150."));
        assert!(v.contains("cc_entrypoint=sdk-cli;"));
        assert!(v.contains("cch="));
        // cch is the first 5 hex chars of sha256(first user text)
        let expected_cch: String = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update("Please summarize this repository status.".as_bytes());
            hex::encode(h.finalize()).chars().take(5).collect()
        };
        assert!(v.contains(&format!("cch={};", expected_cch)));
    }

    #[test]
    fn billing_header_empty_text_is_none() {
        assert!(billing_header_value("").is_none());
    }

    #[test]
    fn build_request_prepends_billing_header_before_identity() {
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(
                        "Please summarize this repository status.".into(),
                    ),
                    name: None,
                },
            )],
            ..Default::default()
        };
        let body = build_request(&req, false);
        assert!(body.system[0].text.starts_with(BILLING_HEADER_MARKER));
        assert_eq!(body.system[1].text, CLAUDE_CODE_SYSTEM);
    }

    #[test]
    fn build_request_does_not_duplicate_billing_header() {
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hi there".into()),
                    name: None,
                },
            )],
            ..Default::default()
        };
        let body = build_request(&req, false);
        let count = body
            .system
            .iter()
            .filter(|b| b.text.contains(BILLING_HEADER_MARKER))
            .count();
        assert_eq!(count, 1);
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
        // system[0] is now the billing header; identity block follows.
        assert!(body.system[0].text.starts_with(BILLING_HEADER_MARKER));
        assert_eq!(body.system[1].text, CLAUDE_CODE_SYSTEM);
    }
}
