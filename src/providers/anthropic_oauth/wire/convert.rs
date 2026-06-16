//! OpenAI chat request -> Anthropic Messages request conversion.

use super::*;
use crate::providers::*;

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
                out.push(AnthropicMessage {
                    role: "user",
                    content: MessageContent::Text(content),
                });
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

                let mut blocks: Vec<RequestContentBlock> = Vec::new();
                if !text.is_empty() {
                    blocks.push(RequestContentBlock::Text { text });
                }
                // Preserve assistant tool calls as Anthropic tool_use blocks so
                // the following tool_result messages can reference them.
                if let Some(tool_calls) = &m.tool_calls {
                    for call in tool_calls {
                        if let async_openai::types::chat::ChatCompletionMessageToolCalls::Function(f) = call {
                            let input: serde_json::Value =
                                serde_json::from_str(&f.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                            blocks.push(RequestContentBlock::ToolUse {
                                id: f.id.clone(),
                                name: f.function.name.clone(),
                                input,
                            });
                        }
                    }
                }

                let content = if blocks.is_empty() {
                    MessageContent::Text(String::new())
                } else {
                    MessageContent::Blocks(blocks)
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
                let result = RequestContentBlock::ToolResult {
                    tool_use_id: m.tool_call_id.clone(),
                    content,
                };
                // Anthropic requires tool_result blocks to live in a user
                // message; merge consecutive tool results into one user turn.
                match out.last_mut() {
                    Some(last)
                        if last.role == "user"
                            && matches!(&last.content, MessageContent::Blocks(b)
                                if b.iter().all(|c| matches!(c, RequestContentBlock::ToolResult { .. }))) =>
                    {
                        if let MessageContent::Blocks(b) = &mut last.content {
                            b.push(result);
                        }
                    }
                    _ => {
                        out.push(AnthropicMessage {
                            role: "user",
                            content: MessageContent::Blocks(vec![result]),
                        });
                    }
                }
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
        tools: convert_tools(request),
        tool_choice: convert_tool_choice(request),
        stream,
    }
}

/// Convert OpenAI tool definitions to Anthropic tool definitions.
fn convert_tools(request: &CreateChatCompletionRequest) -> Option<Vec<ToolDef>> {
    let tools = request.tools.as_ref()?;
    let out: Vec<ToolDef> = tools
        .iter()
        .filter_map(|tool| match tool {
            async_openai::types::chat::ChatCompletionTools::Function(f) => Some(ToolDef {
                name: f.function.name.clone(),
                description: f.function.description.clone(),
                input_schema: f.function.parameters.clone().unwrap_or_else(|| {
                    serde_json::json!({ "type": "object", "properties": {} })
                }),
            }),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Convert an OpenAI tool_choice option to Anthropic's tool_choice JSON.
fn convert_tool_choice(request: &CreateChatCompletionRequest) -> Option<serde_json::Value> {
    use async_openai::types::chat::{ChatCompletionToolChoiceOption, ToolChoiceOptions};
    match request.tool_choice.as_ref()? {
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Auto) => {
            Some(serde_json::json!({ "type": "auto" }))
        }
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Required) => {
            Some(serde_json::json!({ "type": "any" }))
        }
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::None) => None,
        ChatCompletionToolChoiceOption::Function(named) => {
            Some(serde_json::json!({ "type": "tool", "name": named.function.name }))
        }
        _ => None,
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
    fn build_request_includes_tools_and_choice() {
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("weather?".into()),
                    name: None,
                },
            )],
            tools: Some(vec![async_openai::types::chat::ChatCompletionTools::Function(
                async_openai::types::chat::ChatCompletionTool {
                    function: async_openai::types::chat::FunctionObject {
                        name: "get_weather".into(),
                        description: Some("Get weather".into()),
                        parameters: Some(serde_json::json!({
                            "type": "object",
                            "properties": { "city": { "type": "string" } }
                        })),
                        strict: None,
                    },
                },
            )]),
            tool_choice: Some(
                async_openai::types::chat::ChatCompletionToolChoiceOption::Mode(
                    async_openai::types::chat::ToolChoiceOptions::Required,
                ),
            ),
            ..Default::default()
        };
        let body = build_request(&req, false);
        let tools = body.tools.expect("tools set");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "get_weather");
        assert_eq!(body.tool_choice, Some(serde_json::json!({ "type": "any" })));
    }

    #[test]
    fn convert_preserves_tool_use_and_merges_results() {
        let messages = vec![
            ChatCompletionRequestMessage::Assistant(
                async_openai::types::chat::ChatCompletionRequestAssistantMessage {
                    content: None,
                    refusal: None,
                    name: None,
                    audio: None,
                    tool_calls: Some(vec![
                        async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                            async_openai::types::chat::ChatCompletionMessageToolCall {
                                id: "call_1".into(),
                                function: async_openai::types::chat::FunctionCall {
                                    name: "get_weather".into(),
                                    arguments: "{\"city\":\"Paris\"}".into(),
                                },
                            },
                        ),
                    ]),
                    #[allow(deprecated)]
                    function_call: None,
                },
            ),
            ChatCompletionRequestMessage::Tool(
                async_openai::types::chat::ChatCompletionRequestToolMessage {
                    content: async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text("Sunny".into()),
                    tool_call_id: "call_1".into(),
                },
            ),
            ChatCompletionRequestMessage::Tool(
                async_openai::types::chat::ChatCompletionRequestToolMessage {
                    content: async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text("20C".into()),
                    tool_call_id: "call_2".into(),
                },
            ),
        ];
        let (_system, msgs) = convert(&messages);
        assert_eq!(msgs.len(), 2);
        // Assistant tool_use serialized as a tool_use block.
        let asst = serde_json::to_value(&msgs[0]).unwrap();
        assert_eq!(asst["content"][0]["type"], "tool_use");
        assert_eq!(asst["content"][0]["id"], "call_1");
        assert_eq!(asst["content"][0]["input"]["city"], "Paris");
        // Both tool results merged into a single user message.
        let user = serde_json::to_value(&msgs[1]).unwrap();
        assert_eq!(user["role"], "user");
        assert_eq!(user["content"].as_array().unwrap().len(), 2);
        assert_eq!(user["content"][0]["type"], "tool_result");
        assert_eq!(user["content"][1]["tool_use_id"], "call_2");
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
