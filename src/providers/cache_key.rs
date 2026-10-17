use async_openai::types::chat::{
    ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const KEY_PREFIX: &str = "yalr-";
const KEY_HEX_CHARS: usize = 24;
const OPENAI_KEY_MAX_CHARS: usize = 64;
const SYSTEM_PREFIX_CHARS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CacheHint {
    #[default]
    None,
    PromptCacheKey,
    SessionId,
}

impl CacheHint {
    pub fn for_base_url(base_url: &str) -> Self {
        let host = url::Url::parse(base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase));
        match host.as_deref() {
            Some("api.openai.com") => CacheHint::PromptCacheKey,
            Some("openrouter.ai") => CacheHint::SessionId,
            _ => CacheHint::None,
        }
    }

    fn field(self) -> Option<&'static str> {
        match self {
            CacheHint::None => None,
            CacheHint::PromptCacheKey => Some("prompt_cache_key"),
            CacheHint::SessionId => Some("session_id"),
        }
    }
}

fn text_of_user(content: &ChatCompletionRequestUserMessageContent) -> String {
    match content {
        ChatCompletionRequestUserMessageContent::Text(text) => text.clone(),
        ChatCompletionRequestUserMessageContent::Array(parts) => {
            serde_json::to_string(parts).unwrap_or_default()
        }
    }
}

fn leading_system_text(messages: &[ChatCompletionRequestMessage]) -> String {
    let message = messages.iter().find(|m| {
        matches!(
            m,
            ChatCompletionRequestMessage::System(_) | ChatCompletionRequestMessage::Developer(_)
        )
    });
    let serialized = message
        .map(|m| serde_json::to_string(m).unwrap_or_default())
        .unwrap_or_default();
    serialized.chars().take(SYSTEM_PREFIX_CHARS).collect()
}

fn first_user_text(messages: &[ChatCompletionRequestMessage]) -> Option<String> {
    messages.iter().find_map(|m| match m {
        ChatCompletionRequestMessage::User(user) => Some(text_of_user(&user.content)),
        _ => None,
    })
}

pub fn conversation_key(
    messages: &[ChatCompletionRequestMessage],
    client_key: Option<&str>,
) -> Option<String> {
    if let Some(key) = client_key.filter(|k| !k.is_empty()) {
        return Some(key.chars().take(OPENAI_KEY_MAX_CHARS).collect());
    }
    let first_user = first_user_text(messages)?;
    let mut hasher = Sha256::new();
    hasher.update(leading_system_text(messages).as_bytes());
    hasher.update([0u8]);
    hasher.update(first_user.as_bytes());
    let digest = hex::encode(hasher.finalize());
    Some(format!("{KEY_PREFIX}{}", &digest[..KEY_HEX_CHARS]))
}

pub fn apply_cache_hint(
    wire: &mut Value,
    hint: CacheHint,
    messages: &[ChatCompletionRequestMessage],
    client_key: Option<&str>,
) {
    let Some(field) = hint.field() else { return };
    let Some(object) = wire.as_object_mut() else {
        return;
    };
    if object.get(field).is_some_and(|v| !v.is_null()) {
        return;
    }
    if let Some(key) = conversation_key(messages, client_key) {
        insert(object, field, key);
    }
}

fn insert(object: &mut Map<String, Value>, field: &str, key: String) {
    object.insert(field.to_string(), Value::String(key));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ChatRequest;

    fn messages(json: &str) -> Vec<ChatCompletionRequestMessage> {
        let request: ChatRequest = serde_json::from_str(json).unwrap();
        request.messages.clone()
    }

    fn convo(extra: &str) -> Vec<ChatCompletionRequestMessage> {
        messages(&format!(
            r#"{{"model":"gpt-5","messages":[
                {{"role":"system","content":"You are a coding agent."}},
                {{"role":"user","content":"fix the bug"}}{extra}]}}"#
        ))
    }

    #[test]
    fn key_is_stable_as_a_conversation_grows() {
        let first = conversation_key(&convo(""), None).unwrap();
        let later = conversation_key(
            &convo(r#",{"role":"assistant","content":"looking"},{"role":"user","content":"and now?"}"#),
            None,
        )
        .unwrap();
        assert_eq!(first, later);
    }

    #[test]
    fn different_conversations_get_different_keys() {
        let a = conversation_key(&convo(""), None).unwrap();
        let b = conversation_key(
            &messages(r#"{"model":"gpt-5","messages":[{"role":"system","content":"You are a coding agent."},{"role":"user","content":"write docs"}]}"#),
            None,
        )
        .unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_system_prompt_that_grows_at_the_end_keeps_the_key() {
        let long = "x".repeat(2000);
        let a = messages(&format!(r#"{{"model":"m","messages":[{{"role":"system","content":"{long}"}},{{"role":"user","content":"hi"}}]}}"#));
        let b = messages(&format!(r#"{{"model":"m","messages":[{{"role":"system","content":"{long} context-mode active"}},{{"role":"user","content":"hi"}}]}}"#));
        assert_eq!(conversation_key(&a, None), conversation_key(&b, None));
    }

    #[test]
    fn client_key_wins_and_is_clamped_to_openai_limit() {
        let key = "k".repeat(100);
        let got = conversation_key(&convo(""), Some(&key)).unwrap();
        assert_eq!(got.chars().count(), 64);
        assert_eq!(conversation_key(&convo(""), Some("mine")).as_deref(), Some("mine"));
    }

    #[test]
    fn no_user_message_means_no_key() {
        let m = messages(r#"{"model":"m","messages":[{"role":"system","content":"s"}]}"#);
        assert_eq!(conversation_key(&m, None), None);
    }

    #[test]
    fn key_fits_openai_and_openrouter_limits() {
        let key = conversation_key(&convo(""), None).unwrap();
        assert!(key.len() <= 64);
    }

    #[test]
    fn only_known_hosts_get_a_cache_hint() {
        assert_eq!(CacheHint::for_base_url("https://api.openai.com/v1"), CacheHint::PromptCacheKey);
        assert_eq!(CacheHint::for_base_url("https://API.OPENAI.COM/v1"), CacheHint::PromptCacheKey);
        assert_eq!(CacheHint::for_base_url("https://openrouter.ai/api/v1"), CacheHint::SessionId);
        assert_eq!(CacheHint::for_base_url("http://localhost:8001/v1"), CacheHint::None);
        assert_eq!(CacheHint::for_base_url("https://api.openai.com.evil.example/v1"), CacheHint::None);
        assert_eq!(CacheHint::for_base_url("not a url"), CacheHint::None);
    }

    #[test]
    fn hint_sets_the_provider_specific_field() {
        let m = convo("");
        let mut openai = serde_json::json!({"model": "gpt-5"});
        apply_cache_hint(&mut openai, CacheHint::PromptCacheKey, &m, None);
        assert!(openai["prompt_cache_key"].as_str().unwrap().starts_with("yalr-"));
        assert!(openai.get("session_id").is_none());

        let mut openrouter = serde_json::json!({"model": "x/y"});
        apply_cache_hint(&mut openrouter, CacheHint::SessionId, &m, None);
        assert_eq!(openrouter["session_id"], openai["prompt_cache_key"]);
        assert!(openrouter.get("prompt_cache_key").is_none());
    }

    #[test]
    fn none_hint_and_existing_fields_are_left_alone() {
        let m = convo("");
        let mut plain = serde_json::json!({"model": "qwen"});
        apply_cache_hint(&mut plain, CacheHint::None, &m, None);
        assert_eq!(plain, serde_json::json!({"model": "qwen"}));

        let mut set = serde_json::json!({"model": "gpt-5", "prompt_cache_key": "from-client"});
        apply_cache_hint(&mut set, CacheHint::PromptCacheKey, &m, None);
        assert_eq!(set["prompt_cache_key"], "from-client");
    }
}
