use super::*;

const NO_FORCED_TOOL_CHOICE: &[&str] = &["claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-1"];

fn generation(model: &str) -> Option<u32> {
    let mut parts = model.strip_prefix("claude-")?.split('-');
    parts.next().filter(|family| family.chars().all(|c| c.is_ascii_alphabetic()))?;
    parts.next()?.parse().ok()
}

pub fn rejects_sampling_params(model: &str) -> bool {
    generation(model).is_some_and(|major| major >= 5)
}

fn rejects_forced_tool_choice(model: &str) -> bool {
    NO_FORCED_TOOL_CHOICE.iter().any(|prefix| model.starts_with(prefix))
}

fn is_forced(tool_choice: &serde_json::Value) -> bool {
    matches!(
        tool_choice.get("type").and_then(|t| t.as_str()),
        Some("any") | Some("tool")
    )
}

fn drop_blank_text(blocks: &mut Vec<RequestContentBlock>) {
    blocks.retain(|block| !matches!(block, RequestContentBlock::Text { text, .. } if text.trim().is_empty()));
}

pub fn clean_messages(messages: &mut Vec<AnthropicMessage>) {
    for message in messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut message.content {
            drop_blank_text(blocks);
        }
    }
    messages.retain(|m| match &m.content {
        MessageContent::Text(text) => !text.trim().is_empty(),
        MessageContent::Blocks(blocks) => !blocks.is_empty(),
    });
}

pub fn sanitize(body: &mut MessagesRequest) {
    if rejects_sampling_params(&body.model) {
        body.temperature = None;
    }
    if rejects_forced_tool_choice(&body.model) && body.tool_choice.as_ref().is_some_and(is_forced) {
        body.tool_choice = Some(serde_json::json!({ "type": "auto" }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ChatRequest;

    fn body_for(json: &str) -> serde_json::Value {
        let request: ChatRequest = serde_json::from_str(json).unwrap();
        let mut body = build_request(&request, false, CLAUDE_CODE_VERSION);
        sanitize(&mut body);
        serde_json::to_value(&body).unwrap()
    }

    #[test]
    fn temperature_is_dropped_on_generation_five() {
        let wire = body_for(r#"{"model":"claude-opus-5-5","temperature":0.7,"messages":[{"role":"user","content":"hi"}]}"#);
        assert!(wire.get("temperature").is_none());
        let wire = body_for(r#"{"model":"claude-sonnet-5","temperature":0.7,"messages":[{"role":"user","content":"hi"}]}"#);
        assert!(wire.get("temperature").is_none());
        let wire = body_for(r#"{"model":"claude-haiku-4-5","temperature":0.7,"messages":[{"role":"user","content":"hi"}]}"#);
        assert!((wire["temperature"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn forced_tool_choice_falls_back_to_auto_where_rejected() {
        let tools = r#""tools":[{"type":"function","function":{"name":"t","parameters":{"type":"object","properties":{}}}}]"#;
        let wire = body_for(&format!(
            r#"{{"model":"claude-opus-5-5",{tools},"tool_choice":"required","messages":[{{"role":"user","content":"hi"}}]}}"#
        ));
        assert_eq!(wire["tool_choice"]["type"], "auto");
        let wire = body_for(&format!(
            r#"{{"model":"claude-sonnet-5",{tools},"tool_choice":"required","messages":[{{"role":"user","content":"hi"}}]}}"#
        ));
        assert_eq!(wire["tool_choice"]["type"], "any");
    }

    #[test]
    fn blank_messages_are_dropped() {
        let wire = body_for(
            r#"{"model":"claude-opus-5-5","messages":[
                {"role":"user","content":""},
                {"role":"assistant","content":"hi"},
                {"role":"user","content":[{"type":"text","text":"  "},{"type":"text","text":"say ok"}]}
            ]}"#,
        );
        let messages = wire["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["content"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn json_schema_response_format_becomes_output_format() {
        let wire = body_for(
            r#"{"model":"claude-opus-5-5","response_format":{"type":"json_schema","json_schema":{"name":"r","schema":{"type":"object"}}},"messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(wire["output_config"]["format"]["type"], "json_schema");
        assert_eq!(wire["output_config"]["format"]["schema"]["type"], "object");
    }

    #[test]
    fn max_tokens_and_tool_choice_none_carry_over() {
        let wire = body_for(
            r#"{"model":"claude-opus-5-5","max_tokens":20,"tool_choice":"none","messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(wire["max_tokens"], 20);
        assert_eq!(wire["tool_choice"]["type"], "none");
    }
}
