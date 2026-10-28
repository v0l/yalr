//! Anthropic OAuth provider (Claude Pro/Max subscription).
//!
//! Talks to the native Anthropic Messages API using an OAuth Bearer token rather
//! than an `x-api-key`. OAuth tokens require the `anthropic-beta: oauth-2025-04-20`
//! header and, for non-Haiku models, the first system block must be exactly
//! "You are Claude Code, Anthropic's official CLI for Claude." (see `wire`).

mod credits;
#[cfg(test)]
mod stream_tests;
mod wire;

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use super::*;
use crate::db::Database;
use crate::oauth::{OAuthKind, OAuthSession};
use crate::providers::provider_trait::QuotaSnapshot;
use crate::providers::quota::{
    anthropic_quotas_from_headers, anthropic_quotas_from_usage, apply_prepaid_balance, carry_credit,
    has_extra_usage,
};
use wire::*;
pub use wire::{finish_reason, finish_reason_enum, rejects_sampling_params};

const BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,prompt-caching-scope-2026-01-05";
const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_CLIENT_VERSION: &str = CLAUDE_CODE_VERSION;

/// Models advertised when the live model list cannot be fetched.
const FALLBACK_MODELS: &[&str] = &[
    "claude-opus-4-20250514",
    "claude-sonnet-4-20250514",
    "claude-3-7-sonnet-20250219",
    "claude-3-5-sonnet-20241022",
    "claude-3-5-haiku-20241022",
];

pub struct AnthropicOAuthProvider {
    name: String,
    slug: String,
    base_url: String,
    http: reqwest::Client,
    session: Arc<OAuthSession>,
    /// Last quota windows seen on a response, captured from rate-limit headers.
    quota: Arc<RwLock<Vec<QuotaSnapshot>>>,
    models_cache: ModelsCache,
    limits: Arc<RwLock<std::collections::HashMap<String, ListedLimits>>>,
    cc_version: String,
    org_uuid: std::sync::OnceLock<String>,
}

fn user_agent(cc_version: &str) -> String {
    format!("claude-cli/{} (external, cli)", cc_version)
}

impl AnthropicOAuthProvider {
    pub fn new(record: &crate::db::Provider, db: Arc<Database>) -> Self {
        let session = OAuthSession::new(
            OAuthKind::Anthropic,
            record.id,
            db,
            record.oauth_access_token.clone().unwrap_or_default(),
            record.oauth_refresh_token.clone().unwrap_or_default(),
            record.oauth_expires_at.unwrap_or(0),
            None,
        );
        Self {
            name: record.name.clone(),
            slug: record.slug.clone(),
            base_url: record.base_url.trim_end_matches('/').to_string(),
            http: crate::providers::streaming_http_client(),
            session: Arc::new(session),
            quota: Arc::new(RwLock::new(Vec::new())),
            models_cache: ModelsCache::new(),
            limits: Default::default(),
            cc_version: record
                .client_version
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or(CLAUDE_CODE_VERSION)
                .to_string(),
            org_uuid: std::sync::OnceLock::new(),
        }
    }

    fn messages_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/messages", base)
        } else {
            format!("{}/v1/messages", base)
        }
    }

    fn models_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{}/models", base)
        } else {
            format!("{}/v1/models", base)
        }
    }

    /// URL of the OAuth usage endpoint (subscription rate-limit windows).
    ///
    /// This lives at `/api/oauth/usage` on the API host — outside the `/v1`
    /// namespace — so we strip any trailing `/v1` from the configured base URL.
    fn usage_url(&self) -> String {
        let base = self
            .base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .trim_end_matches('/');
        format!("{}/api/oauth/usage", base)
    }

    /// Apply all OAuth-specific headers to a request builder.
    ///
    /// These headers make the request appear to come from the official Claude
    /// Code CLI, which is required for OAuth subscription tokens to be accepted
    /// without the "extra usage" 400 error.
    fn apply_oauth_headers(
        &self,
        builder: reqwest::RequestBuilder,
        token: &str,
    ) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", BETA_HEADER)
            .header("user-agent", user_agent(&self.cc_version))
            .header("x-app", "cli")
            .header("anthropic-dangerous-direct-browser-access", "true")
    }
}

#[async_trait]
impl Provider for AnthropicOAuthProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        if let Some(cached) = self.models_cache.get().await {
            return Ok(cached);
        }

        let token = self
            .session
            .access_token()
            .await
            .map_err(|e| ProviderError::Authentication(e.to_string()))?;
        let resp = self
            .apply_oauth_headers(self.http.get(self.models_url()), &token)
            .send()
            .await;
        if let Ok(r) = resp {
            if r.status().is_success() {
                if let Ok(parsed) = r.json::<crate::providers::ModelListResponse>().await {
                    let listed: std::collections::HashMap<String, ListedLimits> = parsed
                        .data
                        .iter()
                        .filter_map(|m| {
                            let limits = ListedLimits {
                                context_length: m.extra.get("max_input_tokens").and_then(|v| v.as_u64()).map(|v| v as u32),
                                max_output_tokens: m.extra.get("max_tokens").and_then(|v| v.as_u64()).map(|v| v as u32),
                            };
                            (!limits.is_empty()).then(|| (m.id.clone(), limits))
                        })
                        .collect();
                    if let Ok(mut guard) = self.limits.write() {
                        *guard = listed;
                    }
                    let models: Vec<Model> = parsed
                        .data
                        .into_iter()
                        .map(|m| Model {
                            id: m.id,
                            object: "model".to_string(),
                            created: 0,
                            owned_by: "anthropic".to_string(),
                        })
                        .collect();
                    // Only a real upstream listing is cached; the static fallback
                    // below must not pin a stale list after a transient failure.
                    self.models_cache.store(models.clone()).await;
                    return Ok(models);
                }
            }
        }
        Ok(FALLBACK_MODELS
            .iter()
            .map(|id| Model {
                id: id.to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "anthropic".to_string(),
            })
            .collect())
    }

    async fn chat_completions(
        &self,
        request: &ChatRequest,
    ) -> Result<crate::providers::ChatResponse, ProviderError> {
        crate::providers::anthropic_media::check_media(&request.messages)?;
        let token = self
            .session
            .access_token()
            .await
            .map_err(|e| ProviderError::Authentication(e.to_string()))?;
        let mut body = build_request(request, false, &self.cc_version);
        apply_thinking(&mut body, request);
        sanitize(&mut body);
        let body_bytes = serde_json::to_vec(&body).map_err(|e| ProviderError::Other(e.into()))?;
        let encoded = crate::providers::compress::encode_gzip_off_thread(body_bytes.clone()).await;
        let mut upstream = self
            .http
            .post(self.messages_url())
            .header("content-type", "application/json");
        if encoded.gzipped {
            upstream = upstream.header("content-encoding", "gzip");
        }
        let upstream = self
            .apply_oauth_headers(upstream.body(encoded.bytes), &token)
            .build()
            .map_err(|e| ProviderError::Other(e.into()))?;
        let trace = request.trace().cloned();
        if let Some(trace) = &trace {
            trace.upstream_request(
                upstream.url().as_str(),
                crate::request_log::redact::headers(upstream.headers()),
                body_bytes,
            );
        }
        let resp = self
            .http
            .execute(upstream)
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;
        if let Some(trace) = &trace {
            trace.upstream_response(resp.status().as_u16(), crate::request_log::redact::headers(resp.headers()));
        }

        let status = resp.status();
        let quotas = anthropic_quotas_from_headers(resp.headers());
        if !quotas.is_empty() {
            if let Ok(mut guard) = self.quota.write() {
                *guard = carry_credit(&guard, quotas);
            }
        }
        if !status.is_success() {
            let retry_after = wire::retry_after_ms(resp.headers());
            let text = resp.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), retry_after, text));
        }
        let parsed: MessagesResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let (text, tool_calls) = split_content_blocks(&parsed.content, &registered_tool_names(request));
        let usage = parsed.usage.as_ref().map(usage_to_openai);
        if let (Some(trace), Some(raw)) = (&trace, parsed.usage.as_ref()) {
            trace.set_usage(crate::request_log::trace::Usage {
                prompt_tokens: raw.input_tokens + raw.cache_creation_input_tokens + raw.cache_read_input_tokens,
                completion_tokens: raw.output_tokens,
                cached_tokens: raw.cache_read_input_tokens,
                cache_write_tokens: raw.cache_creation_input_tokens,
            });
        }

        let message = async_openai::types::chat::ChatCompletionResponseMessage {
            content: Some(text),
            refusal: None,
            tool_calls,
            annotations: None,
            role: Role::Assistant,
            function_call: None,
            audio: None,
        };
        Ok(CreateChatCompletionResponse {
            id: parsed.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            created: now_secs(),
            model: request.model.clone(),
            choices: vec![async_openai::types::chat::ChatChoice {
                index: 0,
                message,
                finish_reason: finish_reason_enum(parsed.stop_reason.as_deref()),
                logprobs: None,
            }],
            usage,
            service_tier: None,
            #[allow(deprecated)]
            system_fingerprint: None,
            object: "chat.completion".to_string(),
        }
        .into())
    }

    fn chat_completions_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        crate::providers::anthropic_media::check_media(&request.messages)?;
        let mut body = build_request(request, true, &self.cc_version);
        apply_thinking(&mut body, request);
        sanitize(&mut body);
        let trace = request.trace().cloned();
        let user_agent = user_agent(&self.cc_version);
        let url = self.messages_url();
        let http = self.http.clone();
        let session = self.session.clone();
        let model = request.model.clone();
        let registered_tools = registered_tool_names(request);
        let quota_cache = self.quota.clone();

        let stream = async_stream::stream! {
            let token = match session.access_token().await {
                Ok(t) => t,
                Err(e) => {
                    yield Err(ProviderError::Authentication(e.to_string()));
                    return;
                }
            };
            let body_bytes = match serde_json::to_vec(&body) {
                Ok(b) => b,
                Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
            };
            let encoded = crate::providers::compress::encode_gzip_off_thread(body_bytes.clone()).await;
            let mut builder = http.post(&url);
            if encoded.gzipped {
                builder = builder.header("content-encoding", "gzip");
            }
            let upstream = match builder
                .bearer_auth(&token)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("anthropic-beta", BETA_HEADER)
                .header("user-agent", &user_agent)
                .header("x-app", "cli")
                .header("anthropic-dangerous-direct-browser-access", "true")
                .header("accept", "text/event-stream")
                .header("content-type", "application/json")
                .body(encoded.bytes)
                .build()
            {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
            };
            if let Some(trace) = &trace {
                trace.upstream_request(
                    upstream.url().as_str(),
                    crate::request_log::redact::headers(upstream.headers()),
                    body_bytes,
                );
            }
            let resp = match http.execute(upstream).await {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
            };
            if let Some(trace) = &trace {
                trace.upstream_response(resp.status().as_u16(), crate::request_log::redact::headers(resp.headers()));
            }
            let status = resp.status();
            let quotas = anthropic_quotas_from_headers(resp.headers());
            if !quotas.is_empty() {
                if let Ok(mut guard) = quota_cache.write() {
                    *guard = carry_credit(&guard, quotas);
                }
            }
            if !status.is_success() {
                let retry_after = wire::retry_after_ms(resp.headers());
                let text = resp.text().await.unwrap_or_default();
                yield Err(map_status_error(status.as_u16(), retry_after, text));
                return;
            }

            let mut bytes_stream = resp.bytes_stream();
            let mut buffer: Vec<u8> = Vec::new();
            let mut message_id = String::new();
            // Prompt + cache token counts arrive on message_start; output on
            // message_delta. Hold the former to report both together at the end.
            let mut start_usage: Option<AnthropicUsage> = None;
            // Maps an Anthropic content-block index to its OpenAI tool_call index.
            let mut tool_block_indices: std::collections::HashMap<usize, u32> =
                std::collections::HashMap::new();
            let mut next_tool_index: u32 = 0;

            while let Some(chunk) = bytes_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
                };
                buffer.extend_from_slice(&chunk);

                while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                    let raw: Vec<u8> = buffer.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\n', '\r']);
                    let data = match line.strip_prefix("data:") {
                        Some(d) => d.trim(),
                        None => continue,
                    };
                    if data.is_empty() { continue; }
                    let event: StreamEvent = match serde_json::from_str(data) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    if let Some(trace) = &trace {
                        trace.first_event();
                    }
                    match event.event_type.as_str() {
                        "error" => {
                            yield Err(stream_error(event.error));
                            return;
                        }
                        "message_start" => {
                            if let Some(m) = event.message {
                                message_id = m.id.unwrap_or_default();
                                // Anthropic reports input + cache token counts on
                                // message_start; hold them for the final usage chunk.
                                start_usage = m.usage;
                            }
                        }
                        "content_block_start" => {
                            if let (Some(idx), Some(block)) = (event.index, event.content_block) {
                                if block.block_type.as_deref() == Some("tool_use") {
                                    if let (Some(call_id), Some(name)) = (block.id, block.name) {
                                        let tool_index = next_tool_index;
                                        next_tool_index += 1;
                                        tool_block_indices.insert(idx, tool_index);
                                        // Reverse CC canonical name back to the original
                                        // name the caller registered, so upstream sees its
                                        // own tool names.
                                        let original_name = wire::restore_tool_name(&name, &registered_tools);
                                        if let Some(trace) = &trace { trace.first_text(); }
                                        yield Ok(tool_call_start_chunk(
                                            &message_id,
                                            &model,
                                            tool_index,
                                            call_id,
                                            original_name,
                                        ));
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = event.delta {
                                if let Some(text) = delta.text {
                                    if let Some(trace) = &trace { trace.first_text(); }
                                    yield Ok(text_chunk(&message_id, &model, text));
                                } else if let Some(partial_json) = delta.partial_json {
                                    let tool_index = event
                                        .index
                                        .and_then(|i| tool_block_indices.get(&i).copied())
                                        .unwrap_or(0);
                                    yield Ok(tool_call_args_chunk(
                                        &message_id,
                                        &model,
                                        tool_index,
                                        partial_json,
                                    ));
                                } else if let Some(thinking) = delta.thinking {
                                    if let Some(trace) = &trace { trace.first_thinking(); }
                                    yield Ok(wire::reasoning_chunk(&message_id, &model, thinking));
                                }
                            }
                        }
                        "message_delta" => {
                            let fr = event.delta.as_ref().and_then(|d| finish_reason(d.stop_reason.as_deref()));
                            // Merge the prompt/cache counts from message_start with
                            // the output count on this message_delta, then map both
                            // through the shared converter so cache tokens surface.
                            let mut cache_write_tokens = 0u32;
                            let usage = event.usage.map(|delta_usage| {
                                let mut merged = start_usage.clone().unwrap_or_default();
                                merged.output_tokens = delta_usage.output_tokens;
                                if delta_usage.input_tokens > 0 {
                                    merged.input_tokens = delta_usage.input_tokens;
                                }
                                if delta_usage.cache_creation_input_tokens > 0 {
                                    merged.cache_creation_input_tokens = delta_usage.cache_creation_input_tokens;
                                }
                                if delta_usage.cache_read_input_tokens > 0 {
                                    merged.cache_read_input_tokens = delta_usage.cache_read_input_tokens;
                                }
                                cache_write_tokens = merged.cache_creation_input_tokens;
                                usage_to_openai(&merged)
                            });
                            yield Ok(final_chunk(&message_id, &model, fr, usage, cache_write_tokens));
                        }
                        _ => {}
                    }
                }
            }
        };

        Ok(stream.boxed())
    }

    async fn listed_modalities(&self, model: &str) -> Option<ListedModalities> {
        crate::providers::anthropic_media::claude_modalities(model)
    }

    async fn listed_limits(&self, model: &str) -> Option<ListedLimits> {
        let empty = self.limits.read().ok().map_or(true, |g| g.is_empty());
        if empty {
            let _ = self.list_models().await;
        }
        self.limits.read().ok()?.get(model).copied()
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        let token = match self.session.access_token().await {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };
        match self
            .apply_oauth_headers(self.http.get(self.models_url()), &token)
            .send()
            .await
        {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn fetch_quota(&self) -> Option<Vec<QuotaSnapshot>> {
        let cached = || {
            self.quota
                .read()
                .ok()
                .and_then(|g| if g.is_empty() { None } else { Some(g.clone()) })
        };
        let token = match self.session.access_token().await {
            Ok(t) => t,
            Err(_) => return cached(),
        };

        // Preferred source: the dedicated OAuth usage endpoint, which reports
        // per-window utilization (5h / 7d / per-model) plus reset timestamps.
        if let Ok(resp) = self
            .apply_oauth_headers(self.http.get(self.usage_url()), &token)
            .send()
            .await
        {
            if resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                let mut quotas = anthropic_quotas_from_usage(&body);
                if has_extra_usage(&quotas) {
                    if let Some(balance) = self.prepaid_balance(&token).await {
                        apply_prepaid_balance(&mut quotas, balance);
                    }
                }
                if !quotas.is_empty() {
                    if let Ok(mut guard) = self.quota.write() {
                        *guard = quotas.clone();
                    }
                    return Some(quotas);
                }
            }
        }

        // Fallback: scrape rate-limit headers off a lightweight models request.
        match self
            .apply_oauth_headers(self.http.get(self.models_url()), &token)
            .send()
            .await
        {
            Ok(r) => {
                let quotas = anthropic_quotas_from_headers(r.headers());
                if !quotas.is_empty() {
                    let mut guard = self.quota.write().ok()?;
                    *guard = carry_credit(&guard, quotas);
                    Some(guard.clone())
                } else {
                    cached()
                }
            }
            Err(_) => cached(),
        }
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use crate::providers::provider_trait::Provider as _;
    use crate::request_log::Trace;
    use futures::StreamExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":5,\"cache_creation_input_tokens\":20,\"cache_read_input_tokens\":70,\"output_tokens\":1}}}\n\n\
event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n\
event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    async fn provider(server: &MockServer) -> AnthropicOAuthProvider {
        let db = Arc::new(Database::test().await);
        let record = crate::db::Provider {
            id: 1,
            name: "Claude Max".into(),
            slug: "claude-max".into(),
            base_url: format!("{}/v1", server.uri()),
            api_key: None,
            provider_type: crate::db::ProviderType::AnthropicOauth,
            oauth_access_token: Some("sk-ant-oat01-supersecrettokenvalue".into()),
            oauth_refresh_token: Some("refresh".into()),
            oauth_expires_at: Some(i64::MAX / 2),
            oauth_account_id: None,
            client_version: None,
            created_at: Default::default(),
            updated_at: Default::default(),
        };
        AnthropicOAuthProvider::new(&record, db)
    }

    fn request(trace: &Trace) -> ChatRequest {
        let request: ChatRequest = serde_json::from_str(
            r#"{"model":"claude-sonnet-5-5","messages":[{"role":"user","content":"hello"}]}"#,
        )
        .unwrap();
        request.with_trace(trace.clone())
    }

    #[tokio::test]
    async fn model_listing_reports_input_and_output_limits() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "claude-sonnet-5-5", "type": "model", "max_input_tokens": 1_000_000, "max_tokens": 64_000},
                    {"id": "claude-haiku-5-5", "type": "model", "max_input_tokens": 200_000, "max_tokens": null},
                    {"id": "claude-old", "type": "model", "max_input_tokens": null, "max_tokens": null}
                ]
            })))
            .mount(&server)
            .await;
        let provider = provider(&server).await;

        let sonnet = provider.listed_limits("claude-sonnet-5-5").await.unwrap();
        assert_eq!((sonnet.context_length, sonnet.max_output_tokens), (Some(1_000_000), Some(64_000)));
        let haiku = provider.listed_limits("claude-haiku-5-5").await.unwrap();
        assert_eq!((haiku.context_length, haiku.max_output_tokens), (Some(200_000), None));
        assert!(provider.listed_limits("claude-old").await.is_none());
    }

    #[tokio::test]
    async fn stream_fills_the_trace_with_wire_details_and_timings() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .insert_header("anthropic-ratelimit-unified-5h-utilization", "0.42")
                    .set_body_string(SSE),
            )
            .mount(&server)
            .await;
        let provider = provider(&server).await;
        let trace = Trace::new();
        trace.begin_attempt("Claude Max");
        let mut stream = provider.chat_completions_stream(&request(&trace)).unwrap();
        while stream.next().await.is_some() {}

        let upstream = trace.upstream();
        assert!(upstream.url.unwrap().ends_with("/v1/messages"));
        assert_eq!(upstream.status, Some(200));
        let auth = upstream.request_headers.iter().find(|(k, _)| k == "authorization").unwrap();
        assert!(auth.1.contains("..."), "token must be masked: {}", auth.1);
        assert!(!auth.1.contains("supersecrettokenvalue"));
        assert!(upstream.request_headers.iter().any(|(k, v)| k == "x-app" && v == "cli"));
        assert!(upstream
            .response_headers
            .iter()
            .any(|(k, v)| k == "anthropic-ratelimit-unified-5h-utilization" && v == "0.42"));
        let sent: serde_json::Value = serde_json::from_slice(&upstream.request_body.unwrap()).unwrap();
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["model"], "claude-sonnet-5-5");
        assert!(upstream.headers_ms.is_some());
        assert!(upstream.first_event_ms.is_some());
        assert!(upstream.first_thinking_ms.is_some());
        assert!(upstream.first_text_ms.is_some());
        assert!(upstream.first_thinking_ms <= upstream.first_text_ms);
    }

    #[tokio::test]
    async fn non_stream_records_the_exact_bytes_it_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":"msg_1","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":3}}"#,
            ))
            .mount(&server)
            .await;
        let provider = provider(&server).await;
        let trace = Trace::new();
        trace.begin_attempt("Claude Max");
        provider.chat_completions(&request(&trace)).await.unwrap();
        let upstream = trace.upstream();
        let received = server.received_requests().await.unwrap();
        assert_eq!(upstream.request_body.unwrap(), received[0].body);
        assert_eq!(upstream.status, Some(200));
        let usage = trace.usage().unwrap();
        assert_eq!((usage.prompt_tokens, usage.cached_tokens, usage.cache_write_tokens), (5, 0, 0));
    }

    #[tokio::test]
    async fn rate_limit_response_is_still_traced() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "120")
                    .set_body_string(r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#),
            )
            .mount(&server)
            .await;
        let provider = provider(&server).await;
        let trace = Trace::new();
        trace.begin_attempt("Claude Max");
        assert!(provider.chat_completions(&request(&trace)).await.is_err());
        let upstream = trace.upstream();
        assert_eq!(upstream.status, Some(429));
        assert!(upstream.response_headers.iter().any(|(k, v)| k == "retry-after" && v == "120"));
    }
}
