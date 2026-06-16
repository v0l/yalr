//! Request-side conversions: OpenAI chat format -> Anthropic Messages API.

use crate::providers::*;

/// Convert OpenAI chat messages to Anthropic native format.
/// Returns (optional system prompt, message list).
pub(super) fn convert_messages(
    messages: &[ChatCompletionRequestMessage],
) -> (Option<String>, Vec<async_anthropic::types::Message>) {
        let mut system = String::new();
        let mut anthropic_messages: Vec<async_anthropic::types::Message> = Vec::new();

        for msg in messages {
            match msg {
                ChatCompletionRequestMessage::System(sys_msg) => {
                    match &sys_msg.content {
                        ChatCompletionRequestSystemMessageContent::Text(text) => {
                            if !system.is_empty() {
                                system.push('\n');
                            }
                            system.push_str(text);
                        }
                        ChatCompletionRequestSystemMessageContent::Array(parts) => {
                            for part in parts {
                                match part {
                                    async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(text_part) => {
                                        if !system.is_empty() {
                                            system.push('\n');
                                        }
                                        system.push_str(&text_part.text);
                                    }
                                }
                            }
                        }
                    }
                }
                ChatCompletionRequestMessage::User(user_msg) => {
                    let content = match &user_msg.content {
                        ChatCompletionRequestUserMessageContent::Text(text) => text.clone(),
                        ChatCompletionRequestUserMessageContent::Array(parts) => {
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    async_openai::types::chat::ChatCompletionRequestUserMessageContentPart::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    };
                    let message = async_anthropic::types::Message {
                        role: async_anthropic::types::MessageRole::User,
                        content: async_anthropic::types::MessageContentList(vec![
                            async_anthropic::types::MessageContent::Text(
                                async_anthropic::types::Text { text: content },
                            ),
                        ]),
                    };
                    anthropic_messages.push(message);
                }
                ChatCompletionRequestMessage::Assistant(assistant_msg) => {
                    let text = match &assistant_msg.content {
                        Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(t)) => t.clone(),
                        Some(async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Array(parts)) => {
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    async_openai::types::chat::ChatCompletionRequestAssistantMessageContentPart::Text(t) => Some(t.text.clone()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        }
                        None => String::new(),
                    };

                    let mut blocks: Vec<async_anthropic::types::MessageContent> = Vec::new();
                    if !text.is_empty() {
                        blocks.push(async_anthropic::types::MessageContent::Text(
                            async_anthropic::types::Text { text },
                        ));
                    }

                    // Preserve assistant tool calls as Anthropic tool_use blocks so
                    // subsequent tool_result messages can reference them.
                    if let Some(tool_calls) = &assistant_msg.tool_calls {
                        for call in tool_calls {
                            if let async_openai::types::chat::ChatCompletionMessageToolCalls::Function(f) = call {
                                let input: serde_json::Value =
                                    serde_json::from_str(&f.function.arguments)
                                        .unwrap_or(serde_json::Value::Object(Default::default()));
                                blocks.push(async_anthropic::types::MessageContent::ToolUse(
                                    async_anthropic::types::ToolUse {
                                        id: f.id.clone(),
                                        name: f.function.name.clone(),
                                        input,
                                    },
                                ));
                            }
                        }
                    }

                    // Anthropic rejects empty content lists; emit a single empty
                    // text block when an assistant turn has neither text nor tools.
                    if blocks.is_empty() {
                        blocks.push(async_anthropic::types::MessageContent::Text(
                            async_anthropic::types::Text { text: String::new() },
                        ));
                    }

                    anthropic_messages.push(async_anthropic::types::Message {
                        role: async_anthropic::types::MessageRole::Assistant,
                        content: async_anthropic::types::MessageContentList(blocks),
                    });
                }
                ChatCompletionRequestMessage::Tool(tool_msg) => {
                    let content = match &tool_msg.content {
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(t) => t.clone(),
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Array(parts) => {
                            parts.iter().filter_map(|p| match p {
                                async_openai::types::chat::ChatCompletionRequestToolMessageContentPart::Text(t) => Some(t.text.clone()),
                            }).collect::<Vec<_>>().join("\n")
                        }
                    };
                    let tool_result = async_anthropic::types::MessageContent::ToolResult(
                        async_anthropic::types::ToolResult {
                            tool_use_id: tool_msg.tool_call_id.clone(),
                            content: Some(content),
                            is_error: false,
                        },
                    );

                    // Anthropic requires tool_result blocks to live in a user
                    // message. Merge consecutive tool results into one user turn.
                    match anthropic_messages.last_mut() {
                        Some(last)
                            if last.role == async_anthropic::types::MessageRole::User
                                && last
                                    .content
                                    .iter()
                                    .all(|c| matches!(c, async_anthropic::types::MessageContent::ToolResult(_))) =>
                        {
                            last.content.push(tool_result);
                        }
                        _ => {
                            anthropic_messages.push(async_anthropic::types::Message {
                                role: async_anthropic::types::MessageRole::User,
                                content: async_anthropic::types::MessageContentList(vec![tool_result]),
                            });
                        }
                    }
                }
                ChatCompletionRequestMessage::Developer(dev_msg) => {
                    match &dev_msg.content {
                        async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(text) => {
                            if !system.is_empty() {
                                system.push('\n');
                            }
                            system.push_str(text);
                        }
                        async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                            for part in parts {
                                match part {
                                    async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(text_part) => {
                                        if !system.is_empty() {
                                            system.push('\n');
                                        }
                                        system.push_str(&text_part.text);
                                    }
                                }
                            }
                        }
                    }
                }
                ChatCompletionRequestMessage::Function(_) => {
                    tracing::debug!("Skipping unsupported function message type");
                }
            }
        }

        let system = if system.is_empty() {
            None
        } else {
            Some(system)
        };

        (system, anthropic_messages)
    }

pub(super) fn build_client(
    base_url: &str,
    api_key: Option<&str>,
) -> async_anthropic::Client {
    // Simplify: build client chaining setters inline
    let mut builder = async_anthropic::Client::builder();
    if let Some(key) = api_key {
        if !key.is_empty() {
            builder.api_key(key.to_string());
        }
    }
    if !base_url.is_empty() {
        builder.base_url(base_url.to_string());
    }
    builder.build().unwrap_or_default()
}

/// Build an Anthropic create messages request from the OpenAI-format request.
pub(super) fn build_anthropic_request(
    system: Option<String>,
    messages: Vec<async_anthropic::types::Message>,
    request: &CreateChatCompletionRequest,
) -> async_anthropic::types::CreateMessagesRequest {
    let mut builder = async_anthropic::types::CreateMessagesRequestBuilder::default();
    builder.model(&request.model);
    builder.messages(messages);
    builder.max_tokens(request.max_completion_tokens.unwrap_or(4096) as i32);

    if let Some(temp) = request.temperature {
        builder.temperature(temp);
    }

    if let Some(ref stop) = request.stop {
        match stop {
            async_openai::types::chat::StopConfiguration::String(s) => {
                builder.stop_sequences(vec![s.clone()]);
            }
            async_openai::types::chat::StopConfiguration::StringArray(arr) => {
                builder.stop_sequences(arr.clone());
            }
        }
    }

    if let Some(ref metadata) = request.metadata {
        // Convert Metadata struct to serde_json Map
        let map = serde_json::to_value(metadata)
            .ok()
            .and_then(|v| v.as_object().cloned());
        if let Some(m) = map {
            builder.metadata(m);
        }
    }

    if let Some(ref system_content) = system {
        builder.system(system_content.clone());
    }

    if let Some(tools) = convert_tools(request) {
        builder.tools(tools);
    }

    if let Some(tool_choice) = convert_tool_choice(request) {
        builder.tool_choice(tool_choice);
    }

    builder.build().expect("Failed to build Anthropic messages request")
}

/// Convert OpenAI tool definitions to Anthropic tool definitions.
/// Anthropic expects `{ name, description, input_schema }` per tool.
fn convert_tools(
    request: &CreateChatCompletionRequest,
) -> Option<Vec<serde_json::Map<String, serde_json::Value>>> {
    let tools = request.tools.as_ref()?;
    let mut out = Vec::new();
    for tool in tools {
        if let async_openai::types::chat::ChatCompletionTools::Function(f) = tool {
            let mut map = serde_json::Map::new();
            map.insert(
                "name".to_string(),
                serde_json::Value::String(f.function.name.clone()),
            );
            if let Some(desc) = &f.function.description {
                map.insert(
                    "description".to_string(),
                    serde_json::Value::String(desc.clone()),
                );
            }
            let schema = f.function.parameters.clone().unwrap_or_else(|| {
                serde_json::json!({ "type": "object", "properties": {} })
            });
            map.insert("input_schema".to_string(), schema);
            out.push(map);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Convert an OpenAI tool_choice option to Anthropic's tool_choice.
fn convert_tool_choice(
    request: &CreateChatCompletionRequest,
) -> Option<async_anthropic::types::ToolChoice> {
    use async_openai::types::chat::{ChatCompletionToolChoiceOption, ToolChoiceOptions};
    match request.tool_choice.as_ref()? {
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Auto) => {
            Some(async_anthropic::types::ToolChoice::Auto)
        }
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::Required) => {
            Some(async_anthropic::types::ToolChoice::Any)
        }
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::None) => None,
        ChatCompletionToolChoiceOption::Function(named) => {
            Some(async_anthropic::types::ToolChoice::Tool(named.function.name.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_convert_messages_simple_user() {
        let messages = vec![ChatCompletionRequestMessage::User(
            async_openai::types::chat::ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text("Hello world".to_string()),
                name: None,
            },
        )];

        let (system, msgs) = convert_messages(&messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, async_anthropic::types::MessageRole::User);
        assert_eq!(msgs[0].text().as_deref(), Some("Hello world"));
    }

    #[tokio::test]
    async fn test_convert_messages_with_system() {
        let messages = vec![
            ChatCompletionRequestMessage::System(
                async_openai::types::chat::ChatCompletionRequestSystemMessage {
                    content: ChatCompletionRequestSystemMessageContent::Text(
                        "You are helpful".to_string(),
                    ),
                    name: None,
                },
            ),
            ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("Hi".to_string()),
                    name: None,
                },
            ),
        ];

        let (system, msgs) = convert_messages(&messages);
        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn test_convert_assistant_tool_calls_and_tool_result() {
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
                                id: "call_1".to_string(),
                                function: async_openai::types::chat::FunctionCall {
                                    name: "get_weather".to_string(),
                                    arguments: "{\"city\":\"Paris\"}".to_string(),
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
                    content:
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                            "Sunny, 20C".to_string(),
                        ),
                    tool_call_id: "call_1".to_string(),
                },
            ),
        ];

        let (_system, msgs) = convert_messages(&messages);
        assert_eq!(msgs.len(), 2);

        let tool_uses = msgs[0].tool_uses();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].id, "call_1");
        assert_eq!(tool_uses[0].name, "get_weather");
        assert_eq!(tool_uses[0].input["city"], "Paris");

        assert_eq!(msgs[1].role, async_anthropic::types::MessageRole::User);
        match msgs[1].content.first() {
            Some(async_anthropic::types::MessageContent::ToolResult(tr)) => {
                assert_eq!(tr.tool_use_id, "call_1");
                assert_eq!(tr.content.as_deref(), Some("Sunny, 20C"));
            }
            _ => panic!("Expected tool_result content block"),
        }
    }

    #[tokio::test]
    async fn test_convert_multiple_tool_results_merge() {
        let tool = |id: &str| {
            ChatCompletionRequestMessage::Tool(
                async_openai::types::chat::ChatCompletionRequestToolMessage {
                    content:
                        async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                            "ok".to_string(),
                        ),
                    tool_call_id: id.to_string(),
                },
            )
        };
        let (_s, msgs) = convert_messages(&[tool("a"), tool("b")]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.len(), 2);
    }

    #[tokio::test]
    async fn test_build_request_includes_tools() {
        let request = CreateChatCompletionRequest {
            model: "claude-3-haiku-20240307".to_string(),
            messages: vec![],
            tools: Some(vec![
                async_openai::types::chat::ChatCompletionTools::Function(
                    async_openai::types::chat::ChatCompletionTool {
                        function: async_openai::types::chat::FunctionObject {
                            name: "get_weather".to_string(),
                            description: Some("Get weather".to_string()),
                            parameters: Some(serde_json::json!({
                                "type": "object",
                                "properties": { "city": { "type": "string" } }
                            })),
                            strict: None,
                        },
                    },
                ),
            ]),
            tool_choice: Some(
                async_openai::types::chat::ChatCompletionToolChoiceOption::Mode(
                    async_openai::types::chat::ToolChoiceOptions::Required,
                ),
            ),
            ..Default::default()
        };

        let built = build_anthropic_request(None, vec![], &request);
        let tools = built.tools.expect("tools should be set");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "get_weather");
        assert!(tools[0].contains_key("input_schema"));
        assert!(matches!(
            built.tool_choice,
            Some(async_anthropic::types::ToolChoice::Any)
        ));
    }
}
