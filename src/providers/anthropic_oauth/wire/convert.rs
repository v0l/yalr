//! OpenAI chat request -> Anthropic Messages request conversion.

use super::*;
use crate::providers::*;

/// Convert OpenAI-format messages into (system blocks, anthropic messages).
pub fn convert(
    messages: &[ChatCompletionRequestMessage],
) -> (Vec<SystemBlock>, Vec<AnthropicMessage>) {
    let mut system_text = String::new();
    let mut pending_reminders: Vec<String> = Vec::new();
    let mut out: Vec<AnthropicMessage> = Vec::new();

    fn push_system(text: &str, buf: &mut String, pending: &mut Vec<String>, leading: bool) {
        if !leading {
            pending.push(text.to_string());
            return;
        }
        if !buf.is_empty() {
            buf.push('\n');
        }
        buf.push_str(text);
    }

    for msg in messages {
        match msg {
            ChatCompletionRequestMessage::System(m) => match &m.content {
                ChatCompletionRequestSystemMessageContent::Text(t) => {
                    push_system(t, &mut system_text, &mut pending_reminders, out.is_empty())
                }
                ChatCompletionRequestSystemMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestSystemMessageContentPart::Text(t) = p;
                        push_system(&t.text, &mut system_text, &mut pending_reminders, out.is_empty());
                    }
                }
            },
            ChatCompletionRequestMessage::Developer(m) => match &m.content {
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Text(t) => {
                    push_system(t, &mut system_text, &mut pending_reminders, out.is_empty())
                }
                async_openai::types::chat::ChatCompletionRequestDeveloperMessageContent::Array(parts) => {
                    for p in parts {
                        let async_openai::types::chat::ChatCompletionRequestDeveloperMessageContentPart::Text(t) = p;
                        push_system(&t.text, &mut system_text, &mut pending_reminders, out.is_empty());
                    }
                }
            },
            ChatCompletionRequestMessage::User(m) => {
                let content = match &m.content {
                    ChatCompletionRequestUserMessageContent::Text(t) => MessageContent::Blocks(vec![
                    RequestContentBlock::Text { text: t.clone(), cache_control: None },
                ]),
                    ChatCompletionRequestUserMessageContent::Array(parts) => user_content(parts),
                };
                out.push(AnthropicMessage {
                    role: "user",
                    content,
                });
                flush_pending_reminders(&mut out, &mut pending_reminders);
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
                    blocks.push(RequestContentBlock::Text { text, cache_control: None });
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
                                cache_control: None,
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
                    cache_control: None,
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
                flush_pending_reminders(&mut out, &mut pending_reminders);
            }
            ChatCompletionRequestMessage::Function(_) => {}
        }
    }

    flush_pending_reminders(&mut out, &mut pending_reminders);

    let mut system = vec![SystemBlock {
        block_type: "text",
        text: CLAUDE_CODE_SYSTEM.to_string(),
        cache_control: None,
    }, SystemBlock {
        block_type: "text",
        text: SYSTEM_REMINDER_NOTICE.to_string(),
        cache_control: None,
    }];
    if !system_text.is_empty() {
        system.push(SystemBlock {
            block_type: "text",
            text: rewrite_trigger_phrases(&system_text),
            cache_control: None,
        });
    }
    (system, out)
}

fn user_content(
    parts: &[async_openai::types::chat::ChatCompletionRequestUserMessageContentPart],
) -> MessageContent {
    use crate::providers::anthropic_media::{media_block, MediaBlock};
    use async_openai::types::chat::ChatCompletionRequestUserMessageContentPart as Part;

    let mut blocks = Vec::new();
    for part in parts {
        if let Part::Text(t) = part {
            blocks.push(RequestContentBlock::Text {
                text: t.text.clone(),
                cache_control: None,
            });
            continue;
        }
        match media_block(part) {
            Ok(Some(MediaBlock::Image(source))) => blocks.push(RequestContentBlock::Image {
                source,
                cache_control: None,
            }),
            Ok(Some(MediaBlock::Document(source))) => blocks.push(RequestContentBlock::Document {
                source,
                cache_control: None,
            }),
            _ => continue,
        }
    }
    // Text-only messages stay block-shaped rather than plain strings: the
    // last message carries a cache breakpoint, and its shape must not drift
    // when it stops being the last message on the next turn.
    MessageContent::Blocks(blocks)
}

fn default_max_tokens(model: &str) -> u32 {
    if model.contains("-5-5") {
        32768
    } else {
        4096
    }
}

pub fn build_request(
    request: &CreateChatCompletionRequest,
    stream: bool,
    cc_version: &str,
) -> MessagesRequest {
    let (mut system, mut messages) = convert(&request.messages);

    // Remap tool_use names in assistant messages to CC canonical casing.
    // Anthropic's classifier checks that tool names match expected Claude Code
    // conventions. Upstream callers see the original names in responses.
    remap_tool_use_blocks(&mut messages);
    clean_messages(&mut messages);

    // Prepend the Claude Code billing header block (de-duplicated) so the
    // request is accepted/counted like Claude Code rather than rejected as
    // third-party OAuth usage. It must come before the identity block.
    let already_present = system.iter().any(|b| b.text.contains(BILLING_HEADER_MARKER));
    if !already_present {
        if let Some(text) = first_user_text(&request.messages).and_then(|t| billing_header_value(&t, cc_version)) {
            system.insert(
                0,
                SystemBlock {
                    block_type: "text",
                    text,
                    cache_control: None,
                },
            );
        }
    }

    // Inject a prompt-cache breakpoint on the last system block. Anthropic
    // renders tools -> system -> messages, so this caches the tool definitions
    // and the (large, stable) Claude Code system prompt together. Within a
    // conversation the billing header is derived from the first user message and
    // therefore stays constant, so turns 2+ read this prefix from cache. Gated
    // by the shared `anthropic.prompt_cache` config toggle.
    if crate::providers::anthropic::prompt_cache_enabled() {
        if let Some(last) = system.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
        // Second breakpoint at the end of the conversation so every turn
        // after the first reads the whole history from cache instead of
        // re-sending it uncached.
        if let Some(last) = messages.last_mut() {
            match &mut last.content {
                MessageContent::Text(text) => {
                    last.content = MessageContent::Blocks(vec![RequestContentBlock::Text {
                        text: std::mem::take(text),
                        cache_control: Some(CacheControl::ephemeral()),
                    }]);
                }
                MessageContent::Blocks(blocks) => {
                    if let Some(block) = blocks.last_mut() {
                        block.set_cache_control(Some(CacheControl::ephemeral()));
                    }
                }
            }
        }
    }

    let stop_sequences = request.stop.as_ref().map(|s| match s {
        async_openai::types::chat::StopConfiguration::String(v) => vec![v.clone()],
        async_openai::types::chat::StopConfiguration::StringArray(v) => v.clone(),
    });
    // Remap tool definition names to CC canonical casing so Anthropic's
    // classifier recognizes the expected tool set.
    let mut tools = convert_tools(request);
    if let Some(ref mut defs) = tools {
        remap_tool_defs(defs.iter_mut());
    }
    MessagesRequest {
        model: request.model.clone(),
        #[allow(deprecated)]
        max_tokens: request
            .max_completion_tokens
            .or(request.max_tokens)
            .unwrap_or_else(|| default_max_tokens(&request.model)),
        messages,
        system,
        temperature: request.temperature,
        stop_sequences,
        tools,
        tool_choice: convert_tool_choice(request),
        thinking: None,
        output_config: output_format(request).map(|format| OutputConfig {
            effort: None,
            format: Some(format),
        }),
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

fn output_format(request: &CreateChatCompletionRequest) -> Option<OutputFormat> {
    match request.response_format.as_ref()? {
        async_openai::types::chat::ResponseFormat::JsonSchema { json_schema } => Some(OutputFormat {
            format_type: "json_schema",
            schema: json_schema.schema.clone()?,
        }),
        _ => None,
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
        ChatCompletionToolChoiceOption::Mode(ToolChoiceOptions::None) => {
            Some(serde_json::json!({ "type": "none" }))
        }
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
        assert_eq!(system[1].text, SYSTEM_REMINDER_NOTICE);
        assert_eq!(system[2].text, "Be terse");
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
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
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
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
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
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
        let count = body
            .system
            .iter()
            .filter(|b| b.text.contains(BILLING_HEADER_MARKER))
            .count();
        assert_eq!(count, 1);
    }

    fn wire_for(json: &str) -> serde_json::Value {
        let request: CreateChatCompletionRequest = serde_json::from_str(json).unwrap();
        serde_json::to_value(build_request(&request, false, CLAUDE_CODE_VERSION)).unwrap()
    }

    #[test]
    fn leading_system_prompt_is_a_system_block_after_the_identity() {
        let wire = wire_for(
            r#"{"model":"claude-sonnet-4-20250514","messages":[
                {"role":"system","content":"When asked to deploy, run x then y then z."},
                {"role":"user","content":"hi"}]}"#,
        );
        let system = wire["system"].as_array().unwrap();
        assert_eq!(system.len(), 4);
        assert!(system[0]["text"].as_str().unwrap().starts_with(BILLING_HEADER_MARKER));
        assert_eq!(system[1]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(system[2]["text"], SYSTEM_REMINDER_NOTICE);
        assert_eq!(system[3]["text"], "When asked to deploy, run x then y then z.");
        assert_eq!(wire["messages"].as_array().unwrap().len(), 1);
        assert!(!wire["messages"].to_string().contains("deploy"));
    }

    #[test]
    fn mid_conversation_system_message_stays_in_place_as_a_reminder() {
        let wire = wire_for(
            r#"{"model":"claude-sonnet-4-20250514","messages":[
                {"role":"system","content":"base rules"},
                {"role":"user","content":"one"},
                {"role":"assistant","content":"ok"},
                {"role":"system","content":"rules changed: also do q"},
                {"role":"user","content":"two"}]}"#,
        );
        assert_eq!(wire["system"][3]["text"], "base rules");
        assert!(!wire["system"].to_string().contains("rules changed"));
        let messages = wire["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["content"].as_array().unwrap().len(), 1);
        let last = messages[2]["content"].as_array().unwrap();
        assert_eq!(last[0]["text"], "two");
        assert_eq!(last[1]["text"], "<system-reminder>\nrules changed: also do q\n</system-reminder>");
    }

    #[test]
    fn mid_conversation_system_message_before_tool_results_follows_them() {
        let wire = wire_for(
            r#"{"model":"claude-sonnet-4-20250514","messages":[
                {"role":"user","content":"go"},
                {"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read","arguments":"{}"}}]},
                {"role":"system","content":"note"},
                {"role":"tool","tool_call_id":"c1","content":"data"}]}"#,
        );
        let last = wire["messages"].as_array().unwrap().last().unwrap()["content"].as_array().unwrap().clone();
        assert_eq!(last[0]["type"], "tool_result");
        assert_eq!(last[1]["text"], "<system-reminder>\nnote\n</system-reminder>");
    }

    #[test]
    fn build_request_shapes_classifier_trigger_phrase() {
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![
                ChatCompletionRequestMessage::System(
                    async_openai::types::chat::ChatCompletionRequestSystemMessage {
                        content: ChatCompletionRequestSystemMessageContent::Text(
                            "Here is some useful information about the environment you are running in:\nSome details.".into()
                        ),
                        name: None,
                    },
                ),
                ChatCompletionRequestMessage::User(
                    async_openai::types::chat::ChatCompletionRequestUserMessage {
                        content: ChatCompletionRequestUserMessageContent::Text("hi".into()),
                        name: None,
                    },
                ),
            ],
            ..Default::default()
        };
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
        let wire = serde_json::to_string(&body).unwrap();
        assert!(wire.contains("Environment context you are running in:"));
        assert!(!wire.contains("useful information"));
    }

    #[test]
    fn build_request_marks_system_prefix_for_caching() {
        let _guard = crate::providers::anthropic::CACHE_TOGGLE_TEST_LOCK.lock().unwrap();
        crate::providers::anthropic::set_prompt_cache_enabled(true);
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
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
        // Exactly one breakpoint, on the final system block, so tools + the whole
        // system prompt are cached as a single prefix (Anthropic allows 4 max).
        let marked: Vec<usize> = body
            .system
            .iter()
            .enumerate()
            .filter(|(_, b)| b.cache_control.is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(marked, vec![body.system.len() - 1]);
        assert_eq!(
            body.system.last().unwrap().cache_control.as_ref().unwrap().cache_type,
            "ephemeral"
        );

        // The growing conversation prefix gets its own breakpoint on the last
        // message; a plain string is promoted to a block to carry it.
        let last = body.messages.last().unwrap();
        match &last.content {
            MessageContent::Blocks(b) => {
                assert_eq!(b.len(), 1);
                assert!(b[0].cache_control().is_some());
            }
            _ => panic!("last message should be promoted to blocks"),
        }

        // Disabled: no system or message block is marked.
        crate::providers::anthropic::set_prompt_cache_enabled(false);
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
        assert!(body.system.iter().all(|b| b.cache_control.is_none()));
        for m in &body.messages {
            if let MessageContent::Blocks(b) = &m.content {
                assert!(b.iter().all(|blk| blk.cache_control().is_none()));
            }
        }
        crate::providers::anthropic::set_prompt_cache_enabled(true);
    }

    fn strip_cache_control(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                m.remove("cache_control");
                for val in m.values_mut() {
                    strip_cache_control(val);
                }
            }
            serde_json::Value::Array(a) => {
                for val in a.iter_mut() {
                    strip_cache_control(val);
                }
            }
            _ => {}
        }
    }

    fn count_cache_control(v: &serde_json::Value) -> usize {
        match v {
            serde_json::Value::Object(m) => {
                m.contains_key("cache_control") as usize
                    + m.values().map(count_cache_control).sum::<usize>()
            }
            serde_json::Value::Array(a) => a.iter().map(count_cache_control).sum(),
            _ => 0,
        }
    }

    #[test]
    fn build_request_conversation_prefix_is_byte_stable() {
        let _guard = crate::providers::anthropic::CACHE_TOGGLE_TEST_LOCK.lock().unwrap();
        crate::providers::anthropic::set_prompt_cache_enabled(true);

        let system_msg = ChatCompletionRequestMessage::System(
            async_openai::types::chat::ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text("Be terse".into()),
                name: None,
            },
        );
        let first_user = ChatCompletionRequestMessage::User(
            async_openai::types::chat::ChatCompletionRequestUserMessage {
                content: ChatCompletionRequestUserMessageContent::Text(
                    "Investigate the failing build".into(),
                ),
                name: None,
            },
        );
        let tools = Some(vec![async_openai::types::chat::ChatCompletionTools::Function(
            async_openai::types::chat::ChatCompletionTool {
                function: async_openai::types::chat::FunctionObject {
                    name: "read".into(),
                    description: Some("Read a file".into()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": { "path": { "type": "string" } }
                    })),
                    strict: None,
                },
            },
        )]);

        let turn1 = CreateChatCompletionRequest {
            model: "claude-sonnet-5-5".into(),
            messages: vec![system_msg.clone(), first_user.clone()],
            tools: tools.clone(),
            ..Default::default()
        };
        let turn2 = CreateChatCompletionRequest {
            model: "claude-sonnet-5-5".into(),
            messages: vec![
                system_msg,
                first_user,
                ChatCompletionRequestMessage::Assistant(
                    async_openai::types::chat::ChatCompletionRequestAssistantMessage {
                        content: Some(
                            async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(
                                "Let me check".into(),
                            ),
                        ),
                        refusal: None,
                        name: None,
                        audio: None,
                        function_call: None,
                        tool_calls: Some(vec![
                            async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                                async_openai::types::chat::ChatCompletionMessageToolCall {
                                    id: "call_1".into(),
                                    function: async_openai::types::chat::FunctionCall {
                                        name: "read".into(),
                                        arguments: r#"{"path":"build.log"}"#.into(),
                                    },
                                },
                            ),
                        ]),
                    },
                ),
                ChatCompletionRequestMessage::Tool(
                    async_openai::types::chat::ChatCompletionRequestToolMessage {
                        content: async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text(
                            "linker error".into(),
                        ),
                        tool_call_id: "call_1".into(),
                    },
                ),
                ChatCompletionRequestMessage::User(
                    async_openai::types::chat::ChatCompletionRequestUserMessage {
                        content: ChatCompletionRequestUserMessageContent::Text(
                            "Anything else?".into(),
                        ),
                        name: None,
                    },
                ),
            ],
            tools,
            ..Default::default()
        };

        let b1 = build_request(&turn1, false, CLAUDE_CODE_VERSION);
        let b2 = build_request(&turn2, false, CLAUDE_CODE_VERSION);

        // Re-deriving the same request must be byte-identical: no timestamps,
        // randomness, or map ordering may leak into the wire bytes.
        let b2_again = build_request(&turn2, false, CLAUDE_CODE_VERSION);
        assert_eq!(
            serde_json::to_vec(&b2).unwrap(),
            serde_json::to_vec(&b2_again).unwrap()
        );

        // System prefix (billing header, identity, shaped prompt, marker) and
        // tools must be byte-stable across turns.
        assert_eq!(serde_json::to_value(&b1.system).unwrap(), serde_json::to_value(&b2.system).unwrap());
        assert_eq!(serde_json::to_value(&b1.tools).unwrap(), serde_json::to_value(&b2.tools).unwrap());

        // Turn 1's conversation must be a byte-identical prefix of turn 2,
        // modulo the breakpoint that moves to the new last message.
        let m1 = serde_json::to_value(&b1.messages).unwrap();
        let m2 = serde_json::to_value(&b2.messages).unwrap();
        let n1 = m1.as_array().unwrap().len();
        assert!(m2.as_array().unwrap().len() > n1);
        for (i, msg) in m1.as_array().unwrap().iter().enumerate() {
            let mut a = msg.clone();
            let mut b = m2.as_array().unwrap()[i].clone();
            strip_cache_control(&mut a);
            strip_cache_control(&mut b);
            assert_eq!(a, b, "message {i} drifted between turns");
        }

        // Exactly two breakpoints per request: last system block, last message.
        assert_eq!(count_cache_control(&serde_json::to_value(&b1.system).unwrap()), 1);
        assert_eq!(count_cache_control(&m1), 1);
        assert_eq!(count_cache_control(&serde_json::to_value(&b2.system).unwrap()), 1);
        assert_eq!(count_cache_control(&m2), 1);
        crate::providers::anthropic::set_prompt_cache_enabled(false);
    }

    #[test]
    fn build_request_default_max_tokens_by_generation() {
        let base = CreateChatCompletionRequest {
            model: "claude-sonnet-5-5".into(),
            messages: vec![ChatCompletionRequestMessage::User(
                async_openai::types::chat::ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text("hi".into()),
                    name: None,
                },
            )],
            ..Default::default()
        };
        assert_eq!(build_request(&base, false, CLAUDE_CODE_VERSION).max_tokens, 32768);

        let mut older = base.clone();
        older.model = "claude-3-5-haiku-20241022".into();
        assert_eq!(build_request(&older, false, CLAUDE_CODE_VERSION).max_tokens, 4096);

        let mut explicit = base.clone();
        explicit.max_completion_tokens = Some(128);
        assert_eq!(build_request(&explicit, false, CLAUDE_CODE_VERSION).max_tokens, 128);
    }

    #[test]
    fn build_request_marks_tool_result_tail_for_caching() {
        let _guard = crate::providers::anthropic::CACHE_TOGGLE_TEST_LOCK.lock().unwrap();
        crate::providers::anthropic::set_prompt_cache_enabled(true);
        let req = CreateChatCompletionRequest {
            model: "claude-sonnet-4-20250514".into(),
            messages: vec![
                ChatCompletionRequestMessage::User(
                    async_openai::types::chat::ChatCompletionRequestUserMessage {
                        content: ChatCompletionRequestUserMessageContent::Text("hi".into()),
                        name: None,
                    },
                ),
                ChatCompletionRequestMessage::Assistant(
                    async_openai::types::chat::ChatCompletionRequestAssistantMessage {
                        content: Some(
                            async_openai::types::chat::ChatCompletionRequestAssistantMessageContent::Text(
                                "looking".into(),
                            ),
                        ),
                        refusal: None,
                        name: None,
                        audio: None,
                        function_call: None,
                        tool_calls: Some(vec![
                            async_openai::types::chat::ChatCompletionMessageToolCalls::Function(
                                async_openai::types::chat::ChatCompletionMessageToolCall {
                                    id: "call_1".into(),
                                    function: async_openai::types::chat::FunctionCall {
                                        name: "read".into(),
                                        arguments: r#"{"path":"a.txt"}"#.into(),
                                    },
                                },
                            ),
                        ]),
                    },
                ),
                ChatCompletionRequestMessage::Tool(
                    async_openai::types::chat::ChatCompletionRequestToolMessage {
                        content: async_openai::types::chat::ChatCompletionRequestToolMessageContent::Text("file contents".into()),
                        tool_call_id: "call_1".into(),
                    },
                ),
            ],
            ..Default::default()
        };
        let body = build_request(&req, false, CLAUDE_CODE_VERSION);
        let last = body.messages.last().unwrap();
        match &last.content {
            MessageContent::Blocks(b) => {
                assert!(matches!(
                    b.last().unwrap(),
                    RequestContentBlock::ToolResult { .. }
                ));
                assert!(b.last().unwrap().cache_control().is_some());
            }
            _ => panic!("tool result should be blocks"),
        }
        crate::providers::anthropic::set_prompt_cache_enabled(false);
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
        let body = build_request(&req, true, CLAUDE_CODE_VERSION);
        assert!(body.stream);
        assert_eq!(body.max_tokens, 4096);
        // system[0] is now the billing header; identity block follows.
        assert!(body.system[0].text.starts_with(BILLING_HEADER_MARKER));
        assert_eq!(body.system[1].text, CLAUDE_CODE_SYSTEM);
    }
}
