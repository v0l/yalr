//! Wire types and translation between OpenAI chat format and the Anthropic
//! Messages API for the OAuth provider.
//!
//! - [`billing`]: Claude Code identity/billing system blocks.
//! - [`convert`]: OpenAI request -> Anthropic request (messages, tools).
//! - [`chunks`]: Anthropic response/stream -> OpenAI chat output.

mod billing;
mod cc_names;
mod chunks;
mod convert;
mod shaping;

pub use billing::*;
pub use cc_names::{from_cc_name, remap_tool_defs, remap_tool_use_blocks, strip_cc_names};
pub use chunks::*;
pub use convert::*;
pub use shaping::{shape_system_texts, split_assistant_tool_use_messages};

use serde::{Deserialize, Serialize};

use crate::providers::*;

/// Anthropic prompt-cache breakpoint (`{"type":"ephemeral"}`). Attaching it to a
/// system block tells Anthropic to cache everything up to and including that
/// block, so the stable prefix (tools + system prompt) is reused on later
/// requests in the same conversation.
#[derive(Debug, Clone, Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: &'static str,
}

impl CacheControl {
    pub fn ephemeral() -> Self {
        Self { cache_type: "ephemeral" }
    }
}

#[derive(Debug, Serialize)]
pub struct SystemBlock {
    #[serde(rename = "type")]
    pub block_type: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// Anthropic request content: either a plain string or a list of typed blocks.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<RequestContentBlock>),
}

/// Typed content blocks for an outgoing Anthropic message.
#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AnthropicUsage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_creation_input_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
}

/// Convert an Anthropic usage block to OpenAI `CompletionUsage`.
///
/// Anthropic reports `input_tokens` as the *uncached* prompt remainder, with
/// cache-write/read counted separately; OpenAI's `prompt_tokens` is the full
/// prompt, so we sum all three and surface the cache-read subset under
/// `prompt_tokens_details.cached_tokens`.
pub fn usage_to_openai(u: &AnthropicUsage) -> CompletionUsage {
    let prompt_tokens =
        u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
    CompletionUsage {
        prompt_tokens,
        completion_tokens: u.output_tokens,
        total_tokens: prompt_tokens + u.output_tokens,
        completion_tokens_details: None,
        prompt_tokens_details: if u.cache_creation_input_tokens > 0
            || u.cache_read_input_tokens > 0
        {
            Some(async_openai::types::chat::PromptTokensDetails {
                audio_tokens: None,
                cached_tokens: Some(u.cache_read_input_tokens),
            })
        } else {
            None
        },
    }
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
    /// Present on `message_start`; carries the input + cache token counts.
    #[serde(default)]
    pub usage: Option<AnthropicUsage>,
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

pub fn finish_reason(stop: Option<&str>) -> Option<String> {
    match stop {
        Some("end_turn") | Some("max_tokens") | Some("stop_sequence") => Some("stop".to_string()),
        Some("tool_use") => Some("tool_calls".to_string()),
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
        assert_eq!(finish_reason(Some("end_turn")), Some("stop".to_string()));
        assert_eq!(finish_reason(Some("tool_use")), Some("tool_calls".to_string()));
        assert_eq!(finish_reason(None), None);
    }

    #[test]
    fn usage_surfaces_cache_tokens() {
        let u = AnthropicUsage {
            input_tokens: 100,
            output_tokens: 30,
            cache_creation_input_tokens: 20,
            cache_read_input_tokens: 500,
        };
        let mapped = usage_to_openai(&u);
        // prompt_tokens is the full prompt: uncached + cache-write + cache-read.
        assert_eq!(mapped.prompt_tokens, 620);
        assert_eq!(mapped.total_tokens, 650);
        assert_eq!(
            mapped.prompt_tokens_details.and_then(|d| d.cached_tokens),
            Some(500)
        );
    }

    #[test]
    fn usage_no_cache_details_when_uncached() {
        let u = AnthropicUsage {
            input_tokens: 42,
            output_tokens: 7,
            ..Default::default()
        };
        let mapped = usage_to_openai(&u);
        assert_eq!(mapped.prompt_tokens, 42);
        assert!(mapped.prompt_tokens_details.is_none());
    }
}
