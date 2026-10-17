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

pub fn registered_tool_names(request: &crate::providers::ChatRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .flatten()
        .filter_map(|tool| match tool {
            async_openai::types::chat::ChatCompletionTools::Function(f) => Some(f.function.name.clone()),
            _ => None,
        })
        .collect()
}

pub fn restore_tool_name(name: &str, original_names: &[String]) -> String {
    original_names
        .iter()
        .find(|original| original.eq_ignore_ascii_case(name))
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
    fn restore_returns_the_name_the_caller_registered() {
        let originals = vec!["read".to_string(), "Bash".to_string(), "custom_tool".to_string()];
        assert_eq!(restore_tool_name("Read", &originals), "read");
        assert_eq!(restore_tool_name("Bash", &originals), "Bash");
        assert_eq!(restore_tool_name("custom_tool", &originals), "custom_tool");
    }

    #[test]
    fn restore_keeps_unregistered_names() {
        let originals = vec!["read".to_string()];
        assert_eq!(restore_tool_name("Grep", &originals), "Grep");
        assert_eq!(restore_tool_name("Grep", &[]), "Grep");
    }

    #[test]
    fn roundtrip_preserves_registered_names() {
        let originals: Vec<String> = ["read", "Read", "my_custom_function", "MyTool"]
            .iter()
            .map(|n| n.to_string())
            .collect();
        for name in &originals {
            let restored = restore_tool_name(&to_cc_name(name), &originals);
            assert!(restored.eq_ignore_ascii_case(name));
        }
        assert_eq!(restore_tool_name(&to_cc_name("MyTool"), &originals), "MyTool");
    }
}
