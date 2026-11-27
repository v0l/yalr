/// Standard model entry in a /v1/models response (OpenAI-compatible).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelListEntry {
    pub id: String,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u32>,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub max_concurrency: Option<u32>,
    /// Catch-all for provider-specific extra fields on model entries
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Standard /v1/models response wrapper (OpenAI-compatible).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModelListResponse {
    #[serde(default)]
    pub data: Vec<ModelListEntry>,
}

pub mod audio;
pub mod cache_key;
pub mod compress;
pub mod decision;
pub mod effort;
pub mod egress;
pub mod guarded;
pub mod image;
pub mod llamacpp;
pub mod ollama;
pub mod openai;
mod openai_audio;
mod openai_decision;
mod openai_image;
pub mod openrouter;
pub mod request;
pub mod response;
pub mod anthropic;
pub(crate) mod anthropic_media;
pub mod anthropic_oauth;
pub mod openai_oauth;
pub mod provider_trait;
pub mod attribution;
pub mod quota;
pub mod routstr;
pub mod vllm;
pub mod ppq;

/// Build a reqwest client for a provider that may stream long responses.
///
/// Sets a connect timeout (so a dead/hanging upstream fails fast instead of
/// wedging health checks) and a short idle-pool timeout (so infrequent calls
/// don't reuse a server-closed keep-alive socket and hang). Deliberately sets
/// **no** total request `.timeout()` — that would truncate long streaming chat
/// completions. Falls back to the default client if the builder fails.
pub fn streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .pool_idle_timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to build provider HTTP client with timeouts; using default");
            reqwest::Client::new()
        })
}

/// How long a provider's `/v1/models` listing stays fresh.
pub const MODELS_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Shared TTL cache for a provider's model listing.
///
/// Only successful listings are stored, so a provider that is down is retried
/// on the next call instead of serving a stale error.
#[derive(Clone, Default)]
pub struct ModelsCache {
    inner: Arc<tokio::sync::RwLock<Option<(Vec<async_openai::types::models::Model>, std::time::Instant)>>>,
}

impl ModelsCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self) -> Option<Vec<async_openai::types::models::Model>> {
        let guard = self.inner.read().await;
        let (models, cached_at) = guard.as_ref()?;
        (cached_at.elapsed() < MODELS_CACHE_TTL).then(|| models.clone())
    }

    pub async fn store(&self, models: Vec<async_openai::types::models::Model>) {
        *self.inner.write().await = Some((models, std::time::Instant::now()));
    }
}

pub use llamacpp::LlamaCppProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiProvider;
pub use openrouter::OpenRouterProvider;
pub use request::ChatRequest;
pub use response::ChatResponse;
pub use anthropic::AnthropicProvider;
pub use anthropic_oauth::AnthropicOAuthProvider;
pub use openai_oauth::OpenAiOAuthProvider;
pub use routstr::RoutstrProvider;
pub use vllm::VllmProvider;
pub use ppq::PpqProvider;

use crate::db::{Database, Provider as ProviderRecord, ProviderType};
use std::sync::Arc;

pub fn default_client_version(provider_type: ProviderType) -> Option<&'static str> {
    match provider_type {
        ProviderType::AnthropicOauth => Some(anthropic_oauth::DEFAULT_CLIENT_VERSION),
        _ => None,
    }
}


/// Factory function to create a provider instance based on type.
///
/// Returns `None` (with a warning) when the provider can't be constructed —
/// currently only a malformed base URL. Callers skip that provider instead of
/// panicking, so one bad row in the DB can't 500 an API request or kill the
/// config-reload path.
///
/// For OAuth provider types this does NOT have access to the stored tokens and
/// returns `None`; callers handling OAuth providers must use
/// [`create_provider_from_record`] instead.
pub fn create_provider(
    name: &str,
    slug: Option<&str>,
    base_url: &str,
    api_key: Option<&str>,
    provider_type: ProviderType,
) -> Option<Arc<dyn Provider>> {
    build_provider(name, slug, base_url, api_key, provider_type, None)
}

fn build_provider(
    name: &str,
    slug: Option<&str>,
    base_url: &str,
    api_key: Option<&str>,
    provider_type: ProviderType,
    pinned: Option<&egress::Clients>,
) -> Option<Arc<dyn Provider>> {
    fn pin<P>(provider: P, pinned: Option<&egress::Clients>, with: fn(P, &egress::Clients) -> P) -> P {
        match pinned {
            Some(clients) => with(provider, clients),
            None => provider,
        }
    }
    Some(match provider_type {
        ProviderType::OpenAi => Arc::new(pin(OpenAiProvider::new(name, slug, base_url, api_key), pinned, OpenAiProvider::with_clients)),
        ProviderType::LlamaCpp => Arc::new(pin(LlamaCppProvider::new(name, slug, base_url, api_key).ok()?, pinned, LlamaCppProvider::with_clients)),
        ProviderType::Vllm => Arc::new(pin(VllmProvider::new(name, slug, base_url, api_key)?, pinned, VllmProvider::with_clients)),
        ProviderType::Ollama => Arc::new(pin(OllamaProvider::new(name, slug, base_url, api_key).ok()?, pinned, OllamaProvider::with_clients)),
        ProviderType::Routstr => Arc::new(pin(RoutstrProvider::new(name, slug, base_url, api_key), pinned, RoutstrProvider::with_clients)),
        ProviderType::OpenRouter => Arc::new(pin(OpenRouterProvider::new(name, slug, base_url, api_key), pinned, OpenRouterProvider::with_clients)),
        ProviderType::Anthropic => Arc::new(pin(AnthropicProvider::new(name, slug, base_url, api_key), pinned, AnthropicProvider::with_clients)),
        ProviderType::Ppq => Arc::new(pin(PpqProvider::new(name, slug, base_url, api_key), pinned, PpqProvider::with_clients)),
        ProviderType::AnthropicOauth | ProviderType::OpenAiOauth => {
            panic!("OAuth providers must be created via create_provider_from_record")
        }
    })
}

/// Factory that builds a provider from a full DB record, giving OAuth providers
/// access to their stored tokens and a database handle for token refresh.
///
/// Returns `None` (with a warning) if the provider can't be constructed, e.g.
/// a malformed base URL.
pub fn create_provider_from_record(
    record: &ProviderRecord,
    db: Arc<Database>,
) -> Option<Arc<dyn Provider>> {
    let access: guarded::Access = Default::default();
    if let Ok(mut map) = access.write() {
        map.insert(record.slug.clone(), (record.owner_user_id, record.status.clone()));
    }
    create_provider_with_egress(record, db, &egress::Egress::default(), access)
}

pub fn create_user_provider(
    record: &ProviderRecord,
    egress: &egress::Egress,
    access: guarded::Access,
) -> Option<Arc<dyn Provider>> {
    if record.provider_type.is_oauth() {
        tracing::warn!(provider = %record.name, "User owned OAuth provider refused");
        return None;
    }
    let clients = egress.clients();
    let provider = build_provider(
        &record.name,
        Some(&record.slug),
        &record.base_url,
        record.api_key.as_deref(),
        record.provider_type,
        Some(&clients),
    )?;
    Some(guarded::GuardedProvider::wrap(provider, record, egress, access))
}

pub fn create_provider_with_egress(
    record: &ProviderRecord,
    db: Arc<Database>,
    egress: &egress::Egress,
    access: guarded::Access,
) -> Option<Arc<dyn Provider>> {
    if record.owner_user_id.is_some() {
        return create_user_provider(record, egress, access);
    }
    match record.provider_type {
        ProviderType::AnthropicOauth => Some(Arc::new(AnthropicOAuthProvider::new(record, db)) as Arc<dyn Provider>),
        ProviderType::OpenAiOauth => Some(Arc::new(OpenAiOAuthProvider::new(record, db)) as Arc<dyn Provider>),
        other => create_provider(
            &record.name,
            Some(&record.slug),
            &record.base_url,
            record.api_key.as_deref(),
            other,
        ),
    }
}

/// Auto-detect provider type by trying each implementation's get_runtime_info
/// Returns the detected provider type or None if detection fails
pub async fn detect_provider_type(base_url: &str) -> Option<ProviderType> {
    // Try LlamaCpp first (more specific implementation)
    let llamacpp_provider = LlamaCppProvider::new("detect", None, base_url, None).ok()?;
    if let Ok(Some(_info)) = llamacpp_provider.get_runtime_info("dummy-model").await {
        return Some(ProviderType::LlamaCpp);
    }
    
    // If LlamaCpp fails, assume OpenAI (most compatible)
    Some(ProviderType::OpenAi)
}

// Re-export async-openai types for easy swapping
pub use async_openai::types::chat::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
    CreateChatCompletionResponse, CreateChatCompletionStreamResponse,
    CompletionUsage, FinishReason, ChatChoiceLogprobs, Role,
    ChatCompletionMessageToolCallChunk,
};
pub use async_openai::types::models::Model;

pub use provider_trait::*;

/// `extra_fields` key used to carry the prompt-cache *write* (creation) token
/// count out of a provider's streaming usage chunk. OpenAI's `CompletionUsage`
/// has no cache-write field, so providers stash it here and the router consumes
/// and strips it before the chunk reaches the client.
pub const CACHE_WRITE_TOKENS_FIELD: &str = "cache_creation_input_tokens";

/// Custom stream response type that preserves additional fields like reasoning_content
/// from models that support thinking/reasoning content (e.g., Qwen3.5).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingChunk {
    pub id: String,
    pub object: String,
    pub created: u32,
    pub model: String,
    pub choices: Vec<StreamingChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<CompletionUsage>,
    /// Additional fields that may be present in the response (e.g., reasoning_content)
    #[serde(flatten, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingChoice {
    pub index: u32,
    pub delta: StreamingDelta,
    /// Pass-through as raw string. Upstream aggregators (e.g. OpenRouter) forward
    /// provider-specific finish reasons that are NOT in async-openai's strict
    /// `FinishReason` enum (e.g. "eos", "tool_use", "end_turn", "error"). Parsing
    /// those into the enum fails and drops the whole chunk, which surfaces to the
    /// client as a spurious mid-stream stop. We never inspect this value, so keep
    /// it as an opaque string and echo it back verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<ChatChoiceLogprobs>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamingDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatCompletionMessageToolCallChunk>>,
    /// Thinking/reasoning content from models that support it
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Additional fields that may be present in the delta
    #[serde(flatten, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}
