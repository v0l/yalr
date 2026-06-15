//! Anthropic OAuth provider (Claude Pro/Max subscription).
//!
//! Talks to the native Anthropic Messages API using an OAuth Bearer token rather
//! than an `x-api-key`. OAuth tokens require the `anthropic-beta: oauth-2025-04-20`
//! header and, for non-Haiku models, the first system block must be exactly
//! "You are Claude Code, Anthropic's official CLI for Claude." (see `wire`).

mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use super::*;
use crate::db::Database;
use crate::oauth::{OAuthKind, OAuthSession};
use wire::*;

const BETA_HEADER: &str = "oauth-2025-04-20";
const ANTHROPIC_VERSION: &str = "2023-06-01";

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
            http: reqwest::Client::new(),
            session: Arc::new(session),
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
            .http
            .get(self.models_url())
            .bearer_auth(&token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", BETA_HEADER)
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
            .http
            .post(self.messages_url())
            .bearer_auth(&token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", BETA_HEADER)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), text));
        }
        let parsed: MessagesResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let text = parsed
            .content
            .iter()
            .filter_map(|b| b.text.clone())
            .collect::<Vec<_>>()
            .join("");
        let usage = parsed.usage.map(|u| CompletionUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
            completion_tokens_details: None,
            prompt_tokens_details: None,
        });

        let message = async_openai::types::chat::ChatCompletionResponseMessage {
            content: Some(text),
            refusal: None,
            tool_calls: None,
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
                finish_reason: finish_reason(parsed.stop_reason.as_deref()),
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
                .header("accept", "text/event-stream")
                .json(&body)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => { yield Err(ProviderError::Other(e.into())); return; }
            };
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                yield Err(map_status_error(status.as_u16(), text));
                return;
            }

            let mut bytes_stream = resp.bytes_stream();
            let mut buffer = String::new();
            let mut message_id = String::new();

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
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = event.delta {
                                if let Some(text) = delta.text {
                                    yield Ok(text_chunk(&message_id, &model, text));
                                }
                            }
                        }
                        "message_delta" => {
                            let fr = event.delta.as_ref().and_then(|d| finish_reason(d.stop_reason.as_deref()));
                            let usage = event.usage.map(|u| CompletionUsage {
                                prompt_tokens: u.input_tokens,
                                completion_tokens: u.output_tokens,
                                total_tokens: u.input_tokens + u.output_tokens,
                                completion_tokens_details: None,
                                prompt_tokens_details: None,
                            });
                            yield Ok(final_chunk(&message_id, &model, fr, usage));
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
            .http
            .get(self.models_url())
            .bearer_auth(&token)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", BETA_HEADER)
            .send()
            .await
        {
            Ok(r) => Ok(r.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
