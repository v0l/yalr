use super::*;
use crate::providers::ChatRequest;

fn supports_adaptive_thinking(model: &str) -> bool {
    let Some(rest) = model.strip_prefix("claude-") else {
        return false;
    };
    let mut parts = rest.split('-');
    if !parts.next().is_some_and(|family| family.chars().all(|c| c.is_ascii_alphabetic())) {
        return false;
    }
    let major = parts.next().and_then(|p| p.parse::<u32>().ok());
    let minor = parts
        .next()
        .filter(|p| p.len() <= 2)
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    match major {
        Some(major) => major >= 5 || (major == 4 && minor >= 6),
        None => false,
    }
}

fn anthropic_effort(reasoning_effort: &str) -> Option<&'static str> {
    match reasoning_effort {
        "none" | "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

pub fn apply_thinking(body: &mut MessagesRequest, request: &ChatRequest) {
    if !supports_adaptive_thinking(&body.model) {
        return;
    }
    body.thinking = Some(ThinkingConfig {
        thinking_type: "adaptive",
        display: "summarized",
    });
    let effort = request
        .extra_fields()
        .get("reasoning_effort")
        .and_then(|v| v.as_str().map(str::to_string))
        .or_else(|| {
            request
                .reasoning_effort
                .as_ref()
                .and_then(|e| serde_json::to_value(e).ok())
                .and_then(|v| v.as_str().map(str::to_string))
        })
        .and_then(|e| anthropic_effort(&e));
    if effort.is_some() {
        body.output_config
            .get_or_insert(OutputConfig { effort: None, format: None })
            .effort = effort;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_for(json: &str) -> MessagesRequest {
        let request: ChatRequest = serde_json::from_str(json).unwrap();
        let mut body = build_request(&request, true, CLAUDE_CODE_VERSION);
        apply_thinking(&mut body, &request);
        body
    }

    #[test]
    fn adaptive_models_are_recognised_by_version() {
        assert!(supports_adaptive_thinking("claude-opus-5-5"));
        assert!(supports_adaptive_thinking("claude-sonnet-5"));
        assert!(supports_adaptive_thinking("claude-haiku-5-5"));
        assert!(supports_adaptive_thinking("claude-opus-4-6"));
        assert!(!supports_adaptive_thinking("claude-sonnet-4-20250514"));
        assert!(!supports_adaptive_thinking("claude-opus-4-5"));
        assert!(!supports_adaptive_thinking("claude-3-7-sonnet-20250219"));
        assert!(!supports_adaptive_thinking("gpt-5"));
    }

    #[test]
    fn adaptive_model_streams_summarized_thinking() {
        let body = body_for(r#"{"model":"claude-opus-5-5","messages":[{"role":"user","content":"hi"}]}"#);
        let wire = serde_json::to_value(&body).unwrap();
        assert_eq!(wire["thinking"]["type"], "adaptive");
        assert_eq!(wire["thinking"]["display"], "summarized");
        assert!(wire.get("output_config").is_none());
    }

    #[test]
    fn reasoning_effort_maps_to_output_config() {
        let body = body_for(
            r#"{"model":"claude-opus-5-5","reasoning_effort":"high","messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(serde_json::to_value(&body).unwrap()["output_config"]["effort"], "high");
        let body = body_for(
            r#"{"model":"claude-opus-5-5","reasoning_effort":"max","messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(serde_json::to_value(&body).unwrap()["output_config"]["effort"], "max");
    }

    #[test]
    fn older_models_get_no_thinking_fields() {
        let body = body_for(
            r#"{"model":"claude-sonnet-4-20250514","reasoning_effort":"high","messages":[{"role":"user","content":"hi"}]}"#,
        );
        let wire = serde_json::to_value(&body).unwrap();
        assert!(wire.get("thinking").is_none());
        assert!(wire.get("output_config").is_none());
    }
}
