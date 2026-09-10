use async_openai::types::chat::CreateChatCompletionRequest as ChatCompletionRequest;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::ops::{Deref, DerefMut};

/// An inbound chat completion request plus the raw fields that
/// `CreateChatCompletionRequest` cannot carry.
///
/// The typed message structs have no catch-all for unknown keys, so parsing a
/// request silently discards any field we don't model. DeepSeek V4 is broken by
/// exactly that: it rejects a multi-turn request whose assistant turns lack
/// `reasoning_content` ("The `reasoning_content` in the thinking mode must be
/// passed back to the API"), and a client that streams reasoning and replays it
/// hits that as soon as our parse drops it. Keep the raw assistant objects so
/// the OpenAI serializer, which owns the upstream wire shape, can merge them
/// back.
///
/// Derefs to the typed request, so callers read and mutate fields as usual.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    inner: ChatCompletionRequest,
    /// Raw assistant message objects in order, indexed by assistant ordinal
    /// rather than position, so `developer`-to-`system` rewriting can't shift
    /// the mapping.
    assistant_extras: Vec<Map<String, Value>>,
}

impl ChatRequest {
    pub fn from_value(value: Value) -> Result<Self, serde_json::Error> {
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

        Ok(Self {
            inner: serde_json::from_value(value)?,
            assistant_extras,
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
    /// Assistant message at `ordinal`, if it exists.
    pub fn assistant_extras_at(&self, ordinal: usize) -> Option<&Map<String, Value>> {
        self.assistant_extras.get(ordinal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
