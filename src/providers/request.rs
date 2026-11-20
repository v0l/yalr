use async_openai::types::chat::CreateChatCompletionRequest as ChatCompletionRequest;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::ops::{Deref, DerefMut};

/// An inbound chat completion request plus the raw fields that
/// `CreateChatCompletionRequest` cannot carry.
///
/// The typed structs have no catch-all for unknown keys, so parsing a request
/// silently discards anything we don't model. DeepSeek V4 is broken by exactly
/// that: it rejects a multi-turn request whose assistant turns lack
/// `reasoning_content` ("The `reasoning_content` in the thinking mode must be
/// passed back to the API"), and a client that streams reasoning and replays it
/// hits that as soon as our parse drops it. Keep the raw assistant objects so
/// the OpenAI serializer, which owns the upstream wire shape, can merge them
/// back.
///
/// The same applies to top-level request fields: thinking args (`thinking`,
/// `reasoning`, `enable_thinking`, `chat_template_kwargs`, ...) and sampling
/// extras arrive in shapes the typed request has no room for and would vanish
/// on the way upstream, so anything the typed parse does not consume is kept
/// and re-applied at serialization time as well.
///
/// Derefs to the typed request, so callers read and mutate fields as usual.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    inner: ChatCompletionRequest,
    /// Raw assistant message objects in order, indexed by assistant ordinal
    /// rather than position, so `developer`-to-`system` rewriting can't shift
    /// the mapping.
    assistant_extras: Vec<Map<String, Value>>,
    /// Top-level request fields re-applied verbatim on the upstream wire.
    extra_fields: Map<String, Value>,
    body_defaults: Map<String, Value>,
    trace: Option<crate::request_log::Trace>,
    session_header: Option<String>,
    preference: Option<crate::router::preference::Preference>,
}

/// Fields pulled out before the typed parse because their client shapes are
/// wider than the typed field (`modalities` accepts `"image"`, `reasoning_effort`
/// accepts provider values the closed enum would reject with a 400), and
/// `image_config`, which the typed request does not model at all.
const EXTRA_FIELDS: &[&str] = &["modalities", "image_config", "reasoning_effort"];

impl ChatRequest {
    pub fn from_value(mut value: Value) -> Result<Self, serde_json::Error> {
        let mut extra_fields: Map<String, Value> = value
            .as_object_mut()
            .map(|object| {
                EXTRA_FIELDS
                    .iter()
                    .filter_map(|key| object.remove(*key).map(|v| (key.to_string(), v)))
                    .filter(|(_, v)| !v.is_null())
                    .collect()
            })
            .unwrap_or_default();

        let assistant_extras = value
            .get("messages")
            .and_then(|messages| messages.as_array())
            .map(|messages| {
                messages
                    .iter()
                    .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
                    .filter_map(|m| m.as_object().cloned())
                    .collect()
            })
            .unwrap_or_default();

        let mut preference = None;
        if let Some(model) = value.get("model").and_then(Value::as_str) {
            let (bare, suffix) = crate::router::preference::split_model_suffix(model);
            if suffix.is_some() {
                preference = suffix;
                let bare = bare.to_string();
                value["model"] = Value::String(bare);
            }
        }

        let inner = ChatCompletionRequest::deserialize(&value)?;

        // Fields the typed parse consumed are re-serialized faithfully; every
        // other top-level key would be lost, so keep it for the wire.
        let consumed: std::collections::HashSet<String> = serde_json::to_value(&inner)?
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        if let Some(object) = value.as_object() {
            for (key, field) in object {
                if !consumed.contains(key) && !field.is_null() {
                    extra_fields.insert(key.clone(), field.clone());
                }
            }
        }

        if let Some(provider) = extra_fields.get_mut("provider") {
            if let Some(sort) = crate::router::preference::Preference::from_provider_field(Some(&*provider)) {
                preference = Some(sort);
                if let Some(object) = provider.as_object_mut() {
                    object.remove("sort");
                }
            }
            if provider.as_object().is_some_and(|o| o.is_empty()) {
                extra_fields.remove("provider");
            }
        }

        Ok(Self {
            inner,
            assistant_extras,
            extra_fields,
            body_defaults: Map::new(),
            trace: None,
            session_header: None,
            preference,
        })
    }

    /// Top-level fields the typed parse could not carry, re-applied verbatim on
    /// the upstream wire.
    pub fn extra_fields(&self) -> &Map<String, Value> {
        &self.extra_fields
    }

    pub fn preference(&self) -> Option<crate::router::preference::Preference> {
        self.preference
    }

    pub fn input_modalities(&self) -> Vec<crate::router::Modality> {
        use async_openai::types::chat::{
            ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
            ChatCompletionRequestUserMessageContentPart as Part,
        };
        use crate::router::Modality;

        let mut input = Vec::new();
        for message in &self.inner.messages {
            let ChatCompletionRequestMessage::User(user) = message else {
                continue;
            };
            let ChatCompletionRequestUserMessageContent::Array(parts) = &user.content else {
                continue;
            };
            for part in parts {
                let modality = match part {
                    Part::ImageUrl(_) => Modality::Image,
                    Part::InputAudio(_) => Modality::Audio,
                    _ => continue,
                };
                if !input.contains(&modality) {
                    input.push(modality);
                }
            }
        }
        input
    }

    pub fn output_modalities(&self) -> Vec<String> {
        let raw = self
            .extra_fields
            .get("modalities")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect());
        raw.unwrap_or_else(|| {
            self.inner
                .modalities
                .iter()
                .flatten()
                .filter_map(|m| serde_json::to_value(m).ok())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
    }

    /// Raw fields of each assistant message, in assistant-message order.
    pub fn assistant_extras(&self) -> &[Map<String, Value>] {
        &self.assistant_extras
    }

    pub fn inner(&self) -> &ChatCompletionRequest {
        &self.inner
    }
}

impl From<ChatCompletionRequest> for ChatRequest {
    fn from(inner: ChatCompletionRequest) -> Self {
        Self {
            inner,
            assistant_extras: Vec::new(),
            extra_fields: Map::new(),
            body_defaults: Map::new(),
            trace: None,
            session_header: None,
            preference: None,
        }
    }
}

impl<'de> Deserialize<'de> for ChatRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Deref for ChatRequest {
    type Target = ChatCompletionRequest;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for ChatRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl ChatRequest {
    pub fn with_trace(mut self, trace: crate::request_log::Trace) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn with_body_defaults(mut self, defaults: Map<String, Value>) -> Self {
        self.body_defaults = defaults;
        self
    }

    pub fn body_defaults(&self) -> &Map<String, Value> {
        &self.body_defaults
    }

    pub fn requested_effort(&self) -> Option<String> {
        self.extra_fields
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                self.inner
                    .reasoning_effort
                    .as_ref()
                    .and_then(|e| serde_json::to_value(e).ok())
                    .and_then(|v| v.as_str().map(str::to_string))
            })
    }

    pub fn map_reasoning_effort(&mut self, map: &crate::providers::effort::EffortMap) {
        let Some(target) = self.requested_effort().and_then(|asked| map.get(&asked).cloned()) else {
            return;
        };
        self.inner.reasoning_effort = None;
        self.extra_fields.insert("reasoning_effort".to_string(), Value::String(target));
    }

    pub fn trace(&self) -> Option<&crate::request_log::Trace> {
        self.trace.as_ref()
    }

    pub fn with_session_header(mut self, session: Option<String>) -> Self {
        self.session_header = session;
        self
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_header.as_deref().or(self.inner.prompt_cache_key.as_deref())
    }

    /// Assistant message at `ordinal`, if it exists.
    pub fn assistant_extras_at(&self, ordinal: usize) -> Option<&Map<String, Value>> {
        self.assistant_extras.get(ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nitro_suffix_is_stripped_and_read_as_fastest() {
        let request = ChatRequest::from_value(serde_json::json!({"model": "acme/m-1:nitro", "messages": []})).unwrap();
        assert_eq!(request.model, "acme/m-1");
        assert_eq!(request.preference(), Some(crate::router::preference::Preference::Fastest));
    }

    #[test]
    fn provider_sort_is_read_and_kept_off_the_wire_and_beats_the_suffix() {
        let request = ChatRequest::from_value(serde_json::json!({
            "model": "acme/m-1:nitro", "messages": [], "provider": {"sort": "price", "order": ["x"]}
        }))
        .unwrap();
        assert_eq!(request.preference(), Some(crate::router::preference::Preference::Cheapest));
        let provider = request.extra_fields().get("provider").unwrap();
        assert!(provider.get("sort").is_none());
        assert!(provider.get("order").is_some());
    }

    #[test]
    fn a_plain_request_has_no_preference() {
        let request = ChatRequest::from_value(serde_json::json!({"model": "acme/m-1", "messages": []})).unwrap();
        assert_eq!(request.preference(), None);
    }

    fn assistant_count(request: &ChatRequest) -> usize {
        request
            .messages
            .iter()
            .filter(|m| {
                matches!(
                    m,
                    async_openai::types::chat::ChatCompletionRequestMessage::Assistant(_)
                )
            })
            .count()
    }

    #[test]
    fn captures_raw_assistant_messages_in_order() {
        let raw = serde_json::json!({
            "model": "m",
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": null, "reasoning_content": "first"},
                {"role": "tool", "content": "x", "tool_call_id": "1"},
                {"role": "assistant", "content": "second", "reasoning_content": "second"},
            ]
        });

        let request = ChatRequest::from_value(raw).unwrap();

        assert_eq!(assistant_count(&request), 2);
        assert_eq!(
            request.assistant_extras()[0].get("reasoning_content").unwrap(),
            "first"
        );
        assert_eq!(
            request.assistant_extras()[1].get("reasoning_content").unwrap(),
            "second"
        );
    }

    #[test]
    fn typed_parse_still_fails_on_an_invalid_request() {
        let raw = serde_json::json!({"model": "m", "messages": "not-an-array"});
        assert!(ChatRequest::from_value(raw).is_err());
    }

    #[test]
    fn from_typed_request_has_no_extras() {
        let inner = ChatCompletionRequest {
            model: "m".to_string(),
            ..Default::default()
        };
        let request = ChatRequest::from(inner);
        assert!(request.assistant_extras().is_empty());
        assert_eq!(request.model, "m");
    }

    #[test]
    fn image_output_modalities_survive_the_typed_parse() {
        let request = ChatRequest::from_value(serde_json::json!({
            "model": "google/gemini-2.5-flash-image",
            "modalities": ["image", "text"],
            "image_config": {"aspect_ratio": "16:9"},
            "messages": [{"role": "user", "content": "draw a cat"}]
        }))
        .unwrap();

        assert_eq!(request.output_modalities(), vec!["image", "text"]);
        assert_eq!(
            request.extra_fields().get("image_config").unwrap()["aspect_ratio"],
            "16:9"
        );
        assert!(request.modalities.is_none());
    }

    #[test]
    fn thinking_args_survive_the_typed_parse() {
        let request = ChatRequest::from_value(serde_json::json!({
            "model": "kieran/qwen3.8-27b",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "reasoning": {"effort": "high"},
            "enable_thinking": true,
            "chat_template_kwargs": {"enable_thinking": true, "reasoning_effort": "xhigh"},
            "top_k": 40
        }))
        .unwrap();

        assert_eq!(
            request.extra_fields().get("thinking").unwrap()["budget_tokens"],
            2048
        );
        assert_eq!(request.extra_fields().get("reasoning").unwrap()["effort"], "high");
        assert_eq!(request.extra_fields().get("enable_thinking").unwrap(), true);
        assert_eq!(
            request.extra_fields().get("chat_template_kwargs").unwrap()["enable_thinking"],
            true
        );
        assert_eq!(request.extra_fields().get("top_k").unwrap(), 40);
    }

    #[test]
    fn typed_fields_are_not_kept_as_extras() {
        let request = ChatRequest::from_value(serde_json::json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
            "max_tokens": 100
        }))
        .unwrap();

        assert!(request.extra_fields().is_empty());
    }

    #[test]
    fn mapped_reasoning_effort_replaces_the_requested_level() {
        let mut request = ChatRequest::from_value(serde_json::json!({
            "model": "m",
            "messages": [],
            "reasoning_effort": "xhigh"
        }))
        .unwrap();
        let map = [("xhigh".to_string(), "high".to_string())].into_iter().collect();
        request.map_reasoning_effort(&map);
        assert_eq!(request.extra_fields().get("reasoning_effort").unwrap(), "high");
    }

    #[test]
    fn unmapped_reasoning_effort_is_left_alone() {
        let mut request = ChatRequest::from_value(serde_json::json!({
            "model": "m",
            "messages": [],
            "reasoning_effort": "low"
        }))
        .unwrap();
        let map = [("xhigh".to_string(), "high".to_string())].into_iter().collect();
        request.map_reasoning_effort(&map);
        assert_eq!(request.extra_fields().get("reasoning_effort").unwrap(), "low");
    }

    #[test]
    fn typed_reasoning_effort_is_mapped_too() {
        let mut request = ChatRequest::from_value(serde_json::json!({
            "model": "m",
            "messages": []
        }))
        .unwrap();
        request.reasoning_effort = Some(async_openai::types::chat::ReasoningEffort::High);
        let map = [("high".to_string(), "max".to_string())].into_iter().collect();
        request.map_reasoning_effort(&map);
        assert!(request.reasoning_effort.is_none());
        assert_eq!(request.extra_fields().get("reasoning_effort").unwrap(), "max");
    }

    #[test]
    fn custom_reasoning_effort_passes_through_unparsed() {
        let request = ChatRequest::from_value(serde_json::json!({
            "model": "m",
            "messages": [],
            "reasoning_effort": "max"
        }))
        .unwrap();

        assert_eq!(request.extra_fields().get("reasoning_effort").unwrap(), "max");
        assert!(request.reasoning_effort.is_none());
    }

    #[test]
    fn typed_modalities_are_reported_too() {
        let inner: ChatCompletionRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-audio",
            "modalities": ["text", "audio"],
            "messages": []
        }))
        .unwrap();
        assert_eq!(ChatRequest::from(inner).output_modalities(), vec!["text", "audio"]);
    }
}
