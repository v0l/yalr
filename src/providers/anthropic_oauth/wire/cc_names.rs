//! Claude Code canonical tool name remapping.
//!
//! Claude Code uses specific tool name casing (e.g. `Read`, `Bash`, `Grep`).
//! Anthropic's OAuth classifier checks that tool names match expected Claude Code
//! conventions. This module remaps tool names to CC canonical casing on the way
//! in, and reverses them on the way out so upstream callers see their original
//! names.
//!
//! Ported from <https://github.com/shahidshabbir-se/opencode-anthropic-oauth>.

use std::collections::HashMap;
use std::sync::LazyLock;

/// Claude Code canonical tool names (from claude-code / pi-mono / cchistory).
const CC_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

/// Lowercase → CC canonical name.
static CC_LOOKUP: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    CC_TOOLS
        .iter()
        .map(|name| (name.to_lowercase(), *name))
        .collect()
});

/// CC canonical name → lowercase.
static CC_REVERSE: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    CC_TOOLS
        .iter()
        .map(|name| (name.to_string(), name.to_lowercase()))
        .collect()
});

/// Convert a tool name to its Claude Code canonical casing.
///
/// If the lowercase version of `name` matches a known CC tool, returns the
/// canonical casing. Otherwise returns `name` unchanged.
pub fn to_cc_name(name: &str) -> String {
    CC_LOOKUP
        .get(&name.to_lowercase())
        .map(|s| s.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Convert a CC canonical tool name back to lowercase.
///
/// This is the reverse of [`to_cc_name`] — it reverts CC canonical names so
/// upstream callers see the original casing. Non-CC names pass through unchanged.
pub fn from_cc_name(name: &str) -> String {
    // Only reverse if the name matches a CC canonical name exactly
    // (case-insensitive check) AND is not already lowercase.
    CC_REVERSE
        .get(name)
        .filter(|lower| lower != &name)
        .cloned()
        .unwrap_or_else(|| name.to_string())
}

/// Remap tool names in tool definition names to CC canonical casing.
pub fn remap_tool_defs<'a>(
    tools: impl IntoIterator<Item = &'a mut crate::providers::anthropic_oauth::wire::ToolDef>,
) {
    for tool in tools {
        tool.name = to_cc_name(&tool.name);
    }
}

/// Remap tool_use names in message blocks to CC canonical casing.
pub fn remap_tool_use_blocks(
    messages: &mut [crate::providers::anthropic_oauth::wire::AnthropicMessage],
) {
    use crate::providers::anthropic_oauth::wire::{MessageContent, RequestContentBlock};

    for message in messages {
        if message.role != "assistant" {
            continue;
        }
        let blocks = match &mut message.content {
            MessageContent::Blocks(b) => b,
            _ => continue,
        };
        for block in blocks {
            if let RequestContentBlock::ToolUse { name, .. } = block {
                *name = to_cc_name(name);
            }
        }
    }
}

/// Strip CC canonical names from a raw SSE text buffer, reverting tool names
/// back to their original lowercase form in the response stream.
pub fn strip_cc_names(text: &str) -> String {
    let mut result = text.to_string();
    for cc_name in CC_TOOLS.iter() {
        let lower = cc_name.to_lowercase();
        // Normalize both spacing variants to `"name": "toolname"`.
        result = result.replace(
            &format!("\"name\":\"{}\"", cc_name),
            &format!("\"name\": \"{}\"", lower),
        );
        result = result.replace(
            &format!("\"name\": \"{}\"", cc_name),
            &format!("\"name\": \"{}\"", lower),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_cc_known_names() {
        assert_eq!(to_cc_name("read"), "Read");
        assert_eq!(to_cc_name("Read"), "Read");
        assert_eq!(to_cc_name("READ"), "Read");
        assert_eq!(to_cc_name("bash"), "Bash");
        assert_eq!(to_cc_name("BASH"), "Bash");
        assert_eq!(to_cc_name("grep"), "Grep");
        assert_eq!(to_cc_name("webfetch"), "WebFetch");
        assert_eq!(to_cc_name("todowrite"), "TodoWrite");
    }

    #[test]
    fn to_cc_unknown_passthrough() {
        assert_eq!(to_cc_name("custom_tool"), "custom_tool");
        assert_eq!(to_cc_name("MyTool"), "MyTool");
        assert_eq!(to_cc_name(""), "");
    }

    #[test]
    fn from_cc_reverses_known() {
        assert_eq!(from_cc_name("Read"), "read");
        assert_eq!(from_cc_name("Bash"), "bash");
        assert_eq!(from_cc_name("WebFetch"), "webfetch");
    }

    #[test]
    fn from_cc_passthrough_lowercase() {
        assert_eq!(from_cc_name("read"), "read");
        assert_eq!(from_cc_name("bash"), "bash");
    }

    #[test]
    fn from_cc_passthrough_unknown() {
        assert_eq!(from_cc_name("custom_tool"), "custom_tool");
        assert_eq!(from_cc_name("MyTool"), "MyTool");
    }

    #[test]
    fn strip_cc_names_reverts_in_sse() {
        let sse = r#"data: {"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{\"city\":\"Paris\"}"}}"#;
        // No name in this fragment — should be unchanged.
        assert_eq!(strip_cc_names(sse), sse);
    }

    #[test]
    fn strip_cc_names_reverts_tool_name_with_space() {
        let sse = r#"data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"call_1","name": "Read"}}"#;
        let result = strip_cc_names(sse);
        assert!(result.contains(r#""name": "read""#));
        assert!(!result.contains(r#""name": "Read""#));
    }

    #[test]
    fn strip_cc_names_reverts_tool_name_no_space() {
        let sse = r#"data: {"type":"content_block_start","content_block":{"type":"tool_use","id":"call_1","name":"Read"}}"#;
        let result = strip_cc_names(sse);
        assert!(result.contains(r#""name": "read""#));
        assert!(!result.contains(r#""name":"Read""#));
    }

    #[test]
    fn roundtrip_preserves_unknown_names() {
        let name = "my_custom_function";
        assert_eq!(from_cc_name(&to_cc_name(name)), name);
    }
}
