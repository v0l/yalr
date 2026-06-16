//! Claude Code identity and billing system blocks.
//!
//! Anthropic now classifies OAuth (Claude Pro/Max subscription) requests that
//! don't look like Claude Code as "third-party app usage" and rejects them with
//! a 400 disguised as "You're out of extra usage.". To be counted/accepted like
//! Claude Code, we prepend a synthetic `x-anthropic-billing-header` system text
//! block, mirroring what the official CLI sends. Ported from
//! https://github.com/gotgenes/pi-anthropic-auth (src/request-shaping.ts).
//!
//! CLAUDE_CODE_VERSION must be kept roughly in sync with the current Claude Code
//! release. There is no upstream source to import it from; check `claude
//! --version` or https://github.com/anthropics/claude-code. If it drifts too far
//! from what Anthropic expects, OAuth requests may be rejected.

use sha2::{Digest, Sha256};

use crate::providers::*;

/// The Claude Code prefix must be the first system block for OAuth tokens.
pub const CLAUDE_CODE_SYSTEM: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
