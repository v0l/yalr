//! Wire types and translation between OpenAI chat format and the Anthropic
//! Messages API for the OAuth provider.
//!
//! - [`billing`]: Claude Code identity/billing system blocks.
//! - [`convert`]: OpenAI request -> Anthropic request (messages, tools).
//! - [`chunks`]: Anthropic response/stream -> OpenAI chat output.

mod billing;
mod chunks;
mod convert;

pub use billing::*;
pub use chunks::*;
pub use convert::*;

use serde::{Deserialize, Serialize};

use crate::providers::*;

#[derive(Debug, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
}

/// Anthropic request content: either a plain string or a list of typed blocks.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<RequestContentBlock>),
}

/// Typed content blocks for an outgoing Anthropic message.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum RequestContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessage {
    pub role: &'static str,
    pub content: MessageContent,
}

/// Anthropic tool definition: `{ name, description, input_schema }`.
#[derive(Debug, Serialize)]
pub struct ToolDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDef>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
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
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
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
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub content_block: Option<ContentBlock>,
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
    #[serde(default)]
    pub partial_json: Option<String>,
}

pub fn now_secs() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_reason(Some("end_turn")), Some(FinishReason::Stop));
        assert_eq!(finish_reason(Some("tool_use")), Some(FinishReason::ToolCalls));
        assert_eq!(finish_reason(None), None);
    }
}
