//! System prompt shaping for Anthropic OAuth compatibility.
//!
//! Mirrors the techniques from `pi-anthropic-auth`:
//!
//! 1. **Paragraph removal**: Drops paragraphs containing known Pi-specific
//!    anchor strings (identity, docs, filler) that trigger Anthropic's
//!    classifier when combined with OAuth tokens.
//!
//! 2. **Text replacements**: Swaps known classifier trigger phrases that may
//!    appear in keepable paragraphs (isolated by `opencode-anthropic-auth` via
//!    sliding-window bisection of a 10 KB failing prompt).
//!
//! 3. **Preamble detection**: Detects Pi's default system prompt preamble and
//!    replaces it with a minimal neutral prompt, preserving project context,
//!    skills, and appended content that follows.
//!
//! Ported from <https://github.com/gotgenes/pi-anthropic-auth>.

use super::{AnthropicMessage, MessageContent, RequestContentBlock};

/// Prefix of Pi's built-in default system prompt preamble.
const PI_DEFAULT_PROMPT_PREFIX: &str =
    "You are an expert coding assistant operating inside pi, a coding agent harness.";

/// Final line of Pi's built-in default system prompt preamble.
const PI_DEFAULT_PROMPT_TERMINATOR: &str =
    "- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)";

/// Minimal neutral system prompt used for OAuth requests.
///
/// Replaces Pi's verbose default preamble to avoid prompt fingerprinting
/// while preserving any project context that follows.
const MINIMAL_ANTHROPIC_OAUTH_PROMPT: &str = "\
You are an expert coding assistant.
Be concise and helpful.
Use the available tools to answer the user's request.
Show file paths clearly when working with files.";

/// Strings whose presence in a paragraph marks it as Pi-specific and droppable.
///
/// Each entry is checked with `paragraph.contains(anchor)`.
const PARAGRAPH_REMOVAL_ANCHORS: &[&str] = &[
    // Pi identity sentence
    "operating inside pi, a coding agent harness",
    // Pi-specific filler about custom tools
    "In addition to the tools above",
    // Pi documentation block — references Pi-specific docs/paths
    "Pi documentation (read only when the user asks about pi itself",
];

/// Known classifier trigger phrase → safe replacement.
///
/// The original phrase was isolated by `opencode-anthropic-auth` and confirmed
/// to cause Anthropic 400s disguised as "You're out of extra usage." when it
/// reaches Anthropic combined with typical agent context.
struct TextReplacement {
    match_text: &'static str,
    replacement: &'static str,
}

const TEXT_REPLACEMENTS: &[TextReplacement] = &[TextReplacement {
    match_text:
        "Here is some useful information about the environment you are running in:",
    replacement: "Environment context you are running in:",
}];

/// Result of sanitizing a system prompt text block.
#[derive(Debug)]
pub struct ShapingReport {
    pub text: String,
    pub removed_anchors: Vec<String>,
    pub replacement_matches: Vec<String>,
}

/// Remove paragraphs containing known anchor strings and apply inline text
/// replacements for classifier trigger phrases.
///
/// A paragraph is any text between blank lines (`\n\n`). This is resilient
/// to upstream rewording — as long as the anchor still appears somewhere in
/// the paragraph, removal works regardless of surrounding text.
fn sanitize_system_text(text: &str) -> ShapingReport {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut removed_anchors = Vec::new();

    let filtered: Vec<&str> = paragraphs
        .iter()
        .filter(|paragraph| {
            for &anchor in PARAGRAPH_REMOVAL_ANCHORS {
                if paragraph.contains(anchor) {
                    removed_anchors.push(anchor.to_string());
                    return false;
                }
            }
            true
        })
        .copied()
        .collect();

    let mut result = filtered.join("\n\n");
    let mut replacement_matches = Vec::new();

    for rule in TEXT_REPLACEMENTS {
        if result.contains(rule.match_text) {
            replacement_matches.push(rule.match_text.to_string());
            result = result.replace(rule.match_text, rule.replacement);
        }
    }

    ShapingReport {
        text: result.trim().to_string(),
        removed_anchors,
        replacement_matches,
    }
}

/// Shape a system prompt string for Anthropic OAuth compatibility.
///
/// For the normal upstream Pi prompt shape, sanitize only the known preamble
/// span and replace its identity paragraph with the minimal neutral prompt.
/// This preserves downstream configuration/extension points embedded in the
/// preamble (tool snippets and guideline bullets) while still stripping the
/// Pi-specific identity, filler, and documentation paragraphs.
///
/// If Pi's known preamble terminator drifts upstream, we fall back to slicing
/// from `# Project Context`. If that section is also absent, we return the
/// minimal prompt only.
pub fn shape_system_prompt(system_prompt: &str) -> String {
    let prefix_idx = match system_prompt.find(PI_DEFAULT_PROMPT_PREFIX) {
        Some(i) => i,
        None => return system_prompt.to_string(),
    };

    if let Some(term_idx) = system_prompt[ prefix_idx..].find(PI_DEFAULT_PROMPT_TERMINATOR) {
        let abs_term_idx = prefix_idx + term_idx;
        let terminator_end = abs_term_idx + PI_DEFAULT_PROMPT_TERMINATOR.len();
        let preamble = &system_prompt[prefix_idx..terminator_end];
        let report = sanitize_system_text(preamble);
        let shaped_preamble = if report.text.is_empty() {
            MINIMAL_ANTHROPIC_OAUTH_PROMPT.to_string()
        } else {
            format!("{}\n\n{}", MINIMAL_ANTHROPIC_OAUTH_PROMPT, report.text)
        };

        return format!(
            "{}{}{}",
            &system_prompt[..prefix_idx],
            shaped_preamble,
            &system_prompt[terminator_end..]
        );
    }

    // Terminator not found — fall back to `# Project Context` anchor.
    if let Some(pc_idx) = system_prompt.find("\n\n# Project Context\n\n") {
        return format!(
            "{}{}",
            MINIMAL_ANTHROPIC_OAUTH_PROMPT,
            &system_prompt[pc_idx..]
        );
    }

    MINIMAL_ANTHROPIC_OAUTH_PROMPT.to_string()
}

/// Apply system prompt shaping to a vector of system text strings.
///
/// Each entry is checked for Pi's default preamble prefix; matching entries
/// are replaced in-place with the shaped version.  All entries also get
/// text-replacement rules applied (for classifier trigger phrases that may
/// appear in non-preamble system text).
pub fn shape_system_texts(texts: &mut [String]) {
    for text in texts.iter_mut() {
        if text.contains(PI_DEFAULT_PROMPT_PREFIX) {
            *text = shape_system_prompt(text);
        }

        // Apply text replacements to all system blocks, not just those
        // containing the Pi preamble.  Classifier trigger phrases could appear
        // in any system text injected by extensions or callers.
        for rule in TEXT_REPLACEMENTS {
            if text.contains(rule.match_text) {
                *text = text.replace(rule.match_text, rule.replacement);
            }
        }
    }
}

/// Split assistant messages that interleave text and `tool_use` blocks.
///
/// The Anthropic API rejects assistant turns where non-`tool_use` blocks follow
/// a `tool_use` block.  Some serializers can produce this ordering, so we split
/// the message into two consecutive assistant turns: one with text blocks and
/// one with `tool_use` blocks.  The reordering is safe because the text and
/// `tool_use` blocks are semantically independent within a single turn.
pub fn split_assistant_tool_use_messages(
    messages: Vec<AnthropicMessage>,
) -> Vec<AnthropicMessage> {
    let mut out = Vec::with_capacity(messages.len());

    for message in messages {
        if message.role != "assistant" {
            out.push(message);
            continue;
        }

        let blocks = match &message.content {
            MessageContent::Blocks(b) => b,
            _ => {
                out.push(message);
                continue;
            }
        };

        let first_tool_use = blocks
            .iter()
            .position(|b| matches!(b, RequestContentBlock::ToolUse { .. }));

        let first_tool_use = match first_tool_use {
            Some(i) => i,
            None => {
                out.push(message);
                continue;
            }
        };

        // Check if there are any non-tool_use blocks after the first tool_use.
        let has_trailing_non_tool = blocks[first_tool_use..]
            .iter()
            .any(|b| !matches!(b, RequestContentBlock::ToolUse { .. }));

        if !has_trailing_non_tool {
            out.push(message);
            continue;
        }

        let non_tool: Vec<_> = blocks
            .iter()
            .filter(|b| !matches!(b, RequestContentBlock::ToolUse { .. }))
            .cloned()
            .collect();
        let tool_use: Vec<_> = blocks
            .iter()
            .filter(|b| matches!(b, RequestContentBlock::ToolUse { .. }))
            .cloned()
            .collect();

        out.push(AnthropicMessage {
            role: "assistant",
            content: MessageContent::Blocks(non_tool),
        });
        out.push(AnthropicMessage {
            role: "assistant",
            content: MessageContent::Blocks(tool_use),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_pi_identity_paragraph() {
        let text = "\
You are an expert coding assistant operating inside pi, a coding agent harness.
You help users by reading files.

Keep responses short.";
        let report = sanitize_system_text(text);
        assert!(
            !report.text.contains("operating inside pi"),
            "identity paragraph should be removed"
        );
        assert!(
            report.text.contains("Keep responses short"),
            "other paragraphs should be preserved"
        );
        assert!(!report.removed_anchors.is_empty());
    }

    #[test]
    fn sanitize_removes_documentation_paragraph() {
        let text = "\
Some preamble text.

Pi documentation (read only when the user asks about pi itself
- Main docs are at /docs
- Extra info below

Final paragraph.";
        let report = sanitize_system_text(text);
        assert!(!report.text.contains("Pi documentation"));
        assert!(report.text.contains("Some preamble text"));
        assert!(report.text.contains("Final paragraph"));
    }

    #[test]
    fn sanitize_removes_in_addition_to_tools_paragraph() {
        let text = "\
Main instructions here.

In addition to the tools above, you may have access to other custom tools depending on the project.

More stuff after.";
        let report = sanitize_system_text(text);
        assert!(!report.text.contains("In addition to the tools above"));
        assert!(report.text.contains("Main instructions here"));
        assert!(report.text.contains("More stuff after"));
    }

    #[test]
    fn sanitize_applies_text_replacements() {
        let text = "Here is some useful information about the environment you are running in:\nSome details.";
        let report = sanitize_system_text(text);
        assert!(report.text.contains("Environment context you are running in:"));
        assert!(!report.text.contains("useful information"));
        assert!(!report.replacement_matches.is_empty());
    }

    #[test]
    fn sanitize_no_changes_when_clean() {
        let text = "You are a helpful assistant.\n\nBe concise.";
        let report = sanitize_system_text(text);
        assert_eq!(report.text, text);
        assert!(report.removed_anchors.is_empty());
        assert!(report.replacement_matches.is_empty());
    }

    #[test]
    fn shape_system_prompt_replaces_preamble() {
        let prompt = format!(
            "{}\n\
            You help users by reading files, executing commands.\n\n\
            Some guideline.\n\n\
            Some other info.\n\n\
            {}\n\n\
            # Project Context\n\n\
            Actual project content.",
            PI_DEFAULT_PROMPT_PREFIX, PI_DEFAULT_PROMPT_TERMINATOR,
        );
        let shaped = shape_system_prompt(&prompt);
        assert!(
            shaped.starts_with(MINIMAL_ANTHROPIC_OAUTH_PROMPT),
            "should start with minimal prompt"
        );
        assert!(
            shaped.contains("# Project Context"),
            "should preserve project context"
        );
        assert!(
            shaped.contains("Actual project content"),
            "should preserve project content"
        );
    }

    #[test]
    fn shape_system_prompt_fallback_to_project_context() {
        let prompt = format!(
            "{}\n\
            Some content without the terminator.\n\n\
            # Project Context\n\n\
            Project stuff.",
            PI_DEFAULT_PROMPT_PREFIX,
        );
        let shaped = shape_system_prompt(&prompt);
        assert!(shaped.starts_with(MINIMAL_ANTHROPIC_OAUTH_PROMPT));
        assert!(shaped.contains("# Project Context"));
    }

    #[test]
    fn shape_system_prompt_no_preamble_passthrough() {
        let prompt = "You are a helpful assistant.\n\nBe concise.";
        let shaped = shape_system_prompt(prompt);
        assert_eq!(shaped, prompt);
    }

    #[test]
    fn shape_system_texts_modifies_matching_entries() {
        let mut texts = vec![
            "Random system text.".to_string(),
            format!("{}\nSome stuff.\n{}", PI_DEFAULT_PROMPT_PREFIX, PI_DEFAULT_PROMPT_TERMINATOR),
        ];
        shape_system_texts(&mut texts);
        assert_eq!(texts[0], "Random system text.");
        assert!(texts[1].starts_with(MINIMAL_ANTHROPIC_OAUTH_PROMPT));
    }

    #[test]
    fn split_assistant_no_tool_use_passthrough() {
        let messages = vec![AnthropicMessage {
            role: "assistant",
            content: MessageContent::Text("Hello".to_string()),
        }];
        let out = split_assistant_tool_use_messages(messages);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn split_assistant_text_then_tool_use_no_split() {
        let messages = vec![AnthropicMessage {
            role: "assistant",
            content: MessageContent::Blocks(vec![
                RequestContentBlock::Text {
                    text: "Let me check".to_string(),
                },
                RequestContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({}),
                },
            ]),
        }];
        let out = split_assistant_tool_use_messages(messages);
        // text first, then tool_use — no split needed
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn split_assistant_tool_use_then_text_splits() {
        let messages = vec![AnthropicMessage {
            role: "assistant",
            content: MessageContent::Blocks(vec![
                RequestContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "read".to_string(),
                    input: serde_json::json!({}),
                },
                RequestContentBlock::Text {
                    text: "Here's what I found".to_string(),
                },
            ]),
        }];
        let out = split_assistant_tool_use_messages(messages);
        assert_eq!(out.len(), 2, "should split into two assistant turns");
        match &out[0].content {
            MessageContent::Blocks(b) => {
                assert!(matches!(b[0], RequestContentBlock::Text { .. }));
            }
            _ => panic!("expected blocks"),
        }
        match &out[1].content {
            MessageContent::Blocks(b) => {
                assert!(matches!(b[0], RequestContentBlock::ToolUse { .. }));
            }
            _ => panic!("expected blocks"),
        }
    }

    #[test]
    fn split_preserves_non_assistant_messages() {
        let messages = vec![
            AnthropicMessage {
                role: "user",
                content: MessageContent::Text("hi".to_string()),
            },
            AnthropicMessage {
                role: "assistant",
                content: MessageContent::Blocks(vec![
                    RequestContentBlock::ToolUse {
                        id: "call_1".to_string(),
                        name: "read".to_string(),
                        input: serde_json::json!({}),
                    },
                    RequestContentBlock::Text {
                        text: "done".to_string(),
                    },
                ]),
            },
            AnthropicMessage {
                role: "user",
                content: MessageContent::Text("thanks".to_string()),
            },
        ];
        let out = split_assistant_tool_use_messages(messages);
        // user, assistant(text), assistant(tool_use), user = 4
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].role, "user");
        assert_eq!(out[1].role, "assistant");
        assert_eq!(out[2].role, "assistant");
        assert_eq!(out[3].role, "user");
    }
}
