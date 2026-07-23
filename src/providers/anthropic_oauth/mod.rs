//! Anthropic OAuth provider (Claude Pro/Max subscription).
//!
//! Talks to the native Anthropic Messages API using an OAuth Bearer token rather
//! than an `x-api-key`. OAuth tokens require the `anthropic-beta: oauth-2025-04-20`
//! header and, for non-Haiku models, the first system block must be exactly
//! "You are Claude Code, Anthropic's official CLI for Claude." (see `wire`).

mod wire;

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use super::*;
use crate::db::Database;
use crate::oauth::{OAuthKind, OAuthSession};
use crate::providers::provider_trait::QuotaSnapshot;
use crate::providers::quota::{anthropic_quotas_from_headers, anthropic_quotas_from_usage};
use wire::*;

const BETA_HEADER: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,prompt-caching-scope-2026-01-05";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const USER_AGENT: &str = "claude-cli/2.1.81 (external, cli)";

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
            .header("user-agent", USER_AGENT)
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
                    return Ok(parsed
                        .data
                        .into_iter()
                        .map(|m| Model {
                            id: m.id,
                            object: "model".to_string(),
                            created: 0,
                            owned_by: "anthropic".to_string(),
                        })
                        .collect());
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
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let token = self
            .session
            .access_token()
            .await
            .map_err(|e| ProviderError::Authentication(e.to_string()))?;
        let body = build_request(request, false);
        let resp = self
            .apply_oauth_headers(
                self.http.post(self.messages_url()).json(&body),
                &token,
            )
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let status = resp.status();
        let quotas = anthropic_quotas_from_headers(resp.headers());
        if !quotas.is_empty() {
            if let Ok(mut guard) = self.quota.write() {
                *guard = quotas;
            }
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), text));
        }
        let parsed: MessagesResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let (text, tool_calls) = split_content_blocks(&parsed.content);
        let usage = parsed.usage.as_ref().map(usage_to_openai);

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
                finish_reason: match parsed.stop_reason.as_deref() {
                    Some("end_turn") | Some("max_tokens") | Some("stop_sequence") => {
                        Some(async_openai::types::chat::FinishReason::Stop)
                    }
                    Some("tool_use") => Some(async_openai::types::chat::FinishReason::ToolCalls),
                    _ => None,
                },
                logprobs: None,
            }],
            usage,
            service_tier: None,
            #[allow(deprecated)]
            system_fingerprint: None,
            object: "chat.completion".to_string(),
        })
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        let body = build_request(request, true);
        let url = self.messages_url();
        let http = self.http.clone();
        let session = self.session.clone();
        let model = request.model.clone();
        let quota_cache = self.quota.clone();

        let stream = async_stream::stream! {
            let token = match session.access_token().await {
                Ok(t) => t,
                Err(e) => {
                    yield Err(ProviderError::Authentication(e.to_string()));
                    return;
                }
            };
            let resp = http
                .post(&url)
                .bearer_auth(&token)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("anthropic-beta", BETA_HEADER)
                .header("user-agent", USER_AGENT)
                .header("x-app", "cli")
                .header("anthropic-dangerous-direct-browser-access", "true")
                .header("accept", "text/event-stream")
                .json(&body)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
            };
            let status = resp.status();
            let quotas = anthropic_quotas_from_headers(resp.headers());
            if !quotas.is_empty() {
                if let Ok(mut guard) = quota_cache.write() {
                    *guard = quotas;
                }
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                yield Err(map_status_error(status.as_u16(), text));
                return;
            }

            let mut bytes_stream = resp.bytes_stream();
            let mut buffer = String::new();
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
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim_end_matches('\r').to_string();
                    buffer.drain(..=pos);
                    let data = match line.strip_prefix("data:") {
                        Some(d) => d.trim(),
                        None => continue,
                    };
                    if data.is_empty() { continue; }
                    let event: StreamEvent = match serde_json::from_str(data) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    match event.event_type.as_str() {
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
                                        let original_name = wire::from_cc_name(&name);
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
                let quotas = anthropic_quotas_from_usage(&body);
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
                    if let Ok(mut guard) = self.quota.write() {
                        *guard = quotas.clone();
                    }
                    Some(quotas)
                } else {
                    cached()
                }
            }
            Err(_) => cached(),
        }
    }
}
