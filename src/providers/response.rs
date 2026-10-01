use async_openai::types::chat::CreateChatCompletionResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::ops::{Deref, DerefMut};

const UNTYPED_MESSAGE_FIELDS: &[&str] = &["audio"];

#[derive(Debug, Clone)]
pub struct ChatResponse {
    inner: CreateChatCompletionResponse,
    raw_choices: Vec<RawChoice>,
}

#[derive(Debug, Clone, Default)]
struct RawChoice {
    choice: Map<String, Value>,
    message: Map<String, Value>,
}

impl ChatResponse {
    pub fn from_value(mut value: Value) -> Result<Self, serde_json::Error> {
        let raw_choices = value
            .get_mut("choices")
            .and_then(Value::as_array_mut)
            .map(|choices| choices.iter_mut().map(take_raw_choice).collect())
            .unwrap_or_default();
        Ok(Self {
            inner: serde_json::from_value(value)?,
            raw_choices,
        })
    }

    pub fn inner(&self) -> &CreateChatCompletionResponse {
        &self.inner
    }

    pub fn message_field(&self, choice: usize, key: &str) -> Option<&Value> {
        self.raw_choices.get(choice)?.message.get(key)
    }
}

fn take_raw_choice(choice: &mut Value) -> RawChoice {
    let Some(object) = choice.as_object_mut() else {
        return RawChoice::default();
    };
    let mut message = object
        .get("message")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(typed_message) = object.get_mut("message").and_then(Value::as_object_mut) {
        for field in UNTYPED_MESSAGE_FIELDS {
            typed_message.remove(*field);
        }
    }
    message.retain(|_, v| !v.is_null());
    let mut choice = object.clone();
    choice.remove("message");
    choice.retain(|_, v| !v.is_null());
    RawChoice { choice, message }
}

fn fill_missing(target: &mut Map<String, Value>, raw: &Map<String, Value>) {
    for (key, value) in raw {
        if !target.contains_key(key) {
            target.insert(key.clone(), value.clone());
        }
    }
}

impl From<CreateChatCompletionResponse> for ChatResponse {
    fn from(inner: CreateChatCompletionResponse) -> Self {
        Self {
            inner,
            raw_choices: Vec::new(),
        }
    }
}

impl Serialize for ChatResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.inner).map_err(serde::ser::Error::custom)?;
        if let Some(choices) = value.get_mut("choices").and_then(Value::as_array_mut) {
            for (choice, raw) in choices.iter_mut().zip(&self.raw_choices) {
                let Some(choice) = choice.as_object_mut() else {
                    continue;
                };
                fill_missing(choice, &raw.choice);
                if let Some(message) = choice.get_mut("message").and_then(Value::as_object_mut) {
                    fill_missing(message, &raw.message);
                }
            }
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ChatResponse {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_value(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl Deref for ChatResponse {
    type Target = CreateChatCompletionResponse;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for ChatResponse {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENROUTER_IMAGE: &str = r#"{
        "id":"gen-1","object":"chat.completion","created":1,"model":"google/gemini-2.5-flash-image",
        "provider":"Google",
        "choices":[{"index":0,"finish_reason":"stop","native_finish_reason":"STOP",
            "message":{"role":"assistant","content":"here","reasoning":null,
                "images":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}}],
        "usage":{"prompt_tokens":3,"completion_tokens":1290,"total_tokens":1293}
    }"#;

    fn round_trip(body: &str) -> Value {
        let response: ChatResponse = serde_json::from_str(body).unwrap();
        serde_json::to_value(&response).unwrap()
    }

    #[test]
    fn image_output_survives_the_typed_parse() {
        let wire = round_trip(OPENROUTER_IMAGE);
        let message = &wire["choices"][0]["message"];
        assert_eq!(message["images"][0]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(message["content"], "here");
        assert_eq!(wire["choices"][0]["native_finish_reason"], "STOP");
        assert!(message.get("reasoning").is_none());
    }

    #[test]
    fn audio_output_with_a_partial_shape_still_parses() {
        let wire = round_trip(
            r#"{"id":"c","object":"chat.completion","created":1,"model":"gpt-audio",
            "choices":[{"index":0,"finish_reason":"stop",
                "message":{"role":"assistant","content":null,"audio":{"data":"UklGRg==","transcript":"hi"}}}]}"#,
        );
        assert_eq!(wire["choices"][0]["message"]["audio"]["data"], "UklGRg==");
        assert_eq!(wire["choices"][0]["message"]["audio"]["transcript"], "hi");
    }

    #[test]
    fn raw_fields_are_readable() {
        let response: ChatResponse = serde_json::from_str(OPENROUTER_IMAGE).unwrap();
        assert!(response.message_field(0, "images").is_some());
        assert_eq!(response.usage.as_ref().unwrap().completion_tokens, 1290);
    }

    #[test]
    fn typed_response_serializes_unchanged() {
        let typed: CreateChatCompletionResponse = serde_json::from_str(
            r#"{"id":"c","object":"chat.completion","created":1,"model":"m",
            "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"x"}}]}"#,
        )
        .unwrap();
        let expected = serde_json::to_value(&typed).unwrap();
        assert_eq!(serde_json::to_value(ChatResponse::from(typed)).unwrap(), expected);
    }

    #[tokio::test]
    async fn openai_provider_forwards_image_modalities_and_returns_images() {
        use crate::providers::{ChatRequest, OpenAiProvider, Provider};
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "modalities": ["image", "text"],
                "image_config": {"aspect_ratio": "1:1"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_raw(OPENROUTER_IMAGE, "application/json"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let request: ChatRequest = serde_json::from_str(
            r#"{"model":"img","modalities":["image","text"],"image_config":{"aspect_ratio":"1:1"},"messages":[{"role":"user","content":"cat"}]}"#,
        )
        .unwrap();
        let response = provider.chat_completions(&request).await.unwrap();
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(
            wire["choices"][0]["message"]["images"][0]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }
}
