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
mod thinking;

pub use billing::*;
pub use cc_names::{from_cc_name, remap_tool_defs, remap_tool_use_blocks, strip_cc_names};
pub use chunks::*;
pub use convert::*;
pub use shaping::{shape_system_texts, split_assistant_tool_use_messages};
pub use thinking::apply_thinking;

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
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "image")]
    Image {
        source: async_anthropic::types::ContentSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "document")]
    Document {
        source: async_anthropic::types::ContentSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
}

#[derive(Debug, Serialize)]
pub struct AnthropicMessage {
    pub role: &'static str,
    pub content: MessageContent,
}

impl RequestContentBlock {
    pub fn set_cache_control(&mut self, cache_control: Option<CacheControl>) {
        let field = match self {
            RequestContentBlock::Text { cache_control, .. }
            | RequestContentBlock::ToolUse { cache_control, .. }
            | RequestContentBlock::ToolResult { cache_control, .. }
            | RequestContentBlock::Image { cache_control, .. }
            | RequestContentBlock::Document { cache_control, .. } => cache_control,
        };
        *field = cache_control;
    }

    pub fn cache_control(&self) -> Option<&CacheControl> {
        match self {
            RequestContentBlock::Text { cache_control, .. }
            | RequestContentBlock::ToolUse { cache_control, .. }
            | RequestContentBlock::ToolResult { cache_control, .. }
            | RequestContentBlock::Image { cache_control, .. }
            | RequestContentBlock::Document { cache_control, .. } => cache_control.as_ref(),
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_config: Option<OutputConfig>,
    pub stream: bool,
}

#[derive(Debug, Serialize)]
pub struct ThinkingConfig {
    #[serde(rename = "type")]
    pub thinking_type: &'static str,
    pub display: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OutputConfig {
    pub effort: &'static str,
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
    pub thinking: Option<String>,
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

pub fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| secs.saturating_mul(1000))
}

pub fn map_status_error(
    status: u16,
    retry_after: Option<u64>,
    body: String,
) -> ProviderError {
    match status {
        401 | 403 => ProviderError::Authentication(body),
        404 => ProviderError::NotFound(body),
        429 => ProviderError::RateLimit {
            retry_after_ms: retry_after.unwrap_or(2000),
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
    fn retry_after_ms_parses_seconds_header() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("15561"),
        );
        assert_eq!(retry_after_ms(&headers), Some(15_561_000));
    }

    #[test]
    fn retry_after_ms_none_when_absent_or_date_form() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after_ms(&headers), None);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(retry_after_ms(&headers), None);
    }

    #[test]
    fn map_status_error_429_uses_header_retry_after() {
        let err = map_status_error(429, Some(15_561_000), "rate limit".into());
        assert_eq!(err.retry_after_ms(), Some(15_561_000));
        let err = map_status_error(429, None, "rate limit".into());
        assert_eq!(err.retry_after_ms(), Some(2000));
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

    #[test]
    fn image_parts_become_image_blocks_and_text_only_stays_block_shaped() {
        let request: crate::providers::ChatRequest = serde_json::from_str(
            r#"{"model":"claude","messages":[
                {"role":"user","content":[{"type":"text","text":"hi"}]},
                {"role":"assistant","content":"hello"},
                {"role":"user","content":[{"type":"text","text":"look"},{"type":"image_url","image_url":{"url":"https://x.test/a.png"}}]}
            ]}"#,
        )
        .unwrap();
        let body = serde_json::to_value(build_request(&request, false, CLAUDE_CODE_VERSION)).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let users: Vec<_> = messages.iter().filter(|m| m["role"] == "user").collect();
        // Text-only user content is a single text block, never a bare string:
        // the shape must stay stable when this message stops being the last
        // message on the next turn, or the prefix hash misses.
        assert_eq!(users[0]["content"][0]["type"], "text");
        assert_eq!(users[0]["content"][0]["text"], "hi");
        let last = users.last().unwrap()["content"].as_array().unwrap();
        assert!(last.iter().any(|b| b["type"] == "image" && b["source"]["url"] == "https://x.test/a.png"));
    }
}
