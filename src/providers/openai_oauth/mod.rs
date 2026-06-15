//! OpenAI OAuth provider (ChatGPT Plus/Pro via Codex subscription).
//!
//! Calls the ChatGPT/Codex Responses backend with an OAuth Bearer token and the
//! `chatgpt-account-id` header (extracted from the OAuth id_token). Chat
//! completions are translated to/from the Responses API; the backend streams SSE.

mod wire;

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use super::*;
use crate::db::Database;
use crate::oauth::{OAuthKind, OAuthSession};
use wire::*;

const BETA_HEADER: &str = "responses=experimental";
const ORIGINATOR: &str = "codex_cli_rs";

/// Models advertised for ChatGPT/Codex subscription access.
const FALLBACK_MODELS: &[&str] = &["gpt-5", "gpt-5-codex", "codex-mini-latest"];

pub struct OpenAiOAuthProvider {
    name: String,
    slug: String,
    base_url: String,
    http: reqwest::Client,
    session: Arc<OAuthSession>,
}

impl OpenAiOAuthProvider {
    pub fn new(record: &crate::db::Provider, db: Arc<Database>) -> Self {
        let session = OAuthSession::new(
            OAuthKind::OpenAi,
            record.id,
            db,
            record.oauth_access_token.clone().unwrap_or_default(),
            record.oauth_refresh_token.clone().unwrap_or_default(),
            record.oauth_expires_at.unwrap_or(0),
            record.oauth_account_id.clone(),
        );
        Self {
            name: record.name.clone(),
            slug: record.slug.clone(),
            base_url: record.base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
            session: Arc::new(session),
        }
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

#[async_trait]
impl Provider for OpenAiOAuthProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(FALLBACK_MODELS
            .iter()
            .map(|id| Model {
                id: id.to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: "openai".to_string(),
            })
            .collect())
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        // The Codex backend streams; aggregate the stream into a single response.
        let mut stream = self.chat_completions_stream(request)?;
        let mut text = String::new();
        let mut usage = None;
        let mut id = uuid::Uuid::new_v4().to_string();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if !chunk.id.is_empty() {
                id = chunk.id.clone();
            }
            if let Some(choice) = chunk.choices.first() {
                if let Some(c) = &choice.delta.content {
                    text.push_str(c);
                }
            }
            if chunk.usage.is_some() {
                usage = chunk.usage.clone();
            }
        }

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
            id,
            created: now_secs(),
            model: request.model.clone(),
            choices: vec![async_openai::types::chat::ChatChoice {
                index: 0,
                message,
                finish_reason: Some(FinishReason::Stop),
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
        let url = self.responses_url();
        let http = self.http.clone();
        let session = self.session.clone();
        let model = request.model.clone();

        let stream = async_stream::stream! {
            let token = match session.access_token().await {
                Ok(t) => t,
                Err(e) => { yield Err(ProviderError::Authentication(e.to_string())); return; }
            };
            let mut req = http
                .post(&url)
                .bearer_auth(&token)
                .header("OpenAI-Beta", BETA_HEADER)
                .header("originator", ORIGINATOR)
                .header("session_id", uuid::Uuid::new_v4().to_string())
                .header("accept", "text/event-stream")
                .json(&body);
            if let Some(account_id) = session.account_id().await {
                req = req.header("chatgpt-account-id", account_id);
            }

            let resp = match req.send().await {
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
            let mut response_id = String::new();
            let mut emitted_text = false;

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
                    if data.is_empty() || data == "[DONE]" { continue; }
                    let event: StreamEvent = match serde_json::from_str(data) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    match event.event_type.as_str() {
                        "response.created" | "response.in_progress" => {
                            if let Some(r) = &event.response {
                                if let Some(rid) = &r.id { response_id = rid.clone(); }
                            }
                        }
                        "response.output_text.delta" => {
                            if let Some(delta) = event.delta {
                                emitted_text = true;
                                yield Ok(text_chunk(&response_id, &model, delta));
                            }
                        }
                        "response.completed" => {
                            // Fallback: if the backend didn't stream text deltas,
                            // emit the aggregated output text from the final object.
                            if !emitted_text {
                                if let Some(r) = &event.response {
                                    let text = r.output_text();
                                    if !text.is_empty() {
                                        yield Ok(text_chunk(&response_id, &model, text));
                                    }
                                }
                            }
                            let usage = event.response
                                .as_ref()
                                .and_then(|r| r.usage.as_ref())
                                .map(usage_to_openai);
                            yield Ok(final_chunk(&response_id, &model, usage));
                        }
                        "response.failed" | "error" => {
                            yield Err(ProviderError::ServerError {
                                message: "ChatGPT backend reported an error".to_string(),
                                status_code: None,
                            });
                            return;
                        }
                        _ => {}
                    }
                }
            }
        };

        Ok(stream.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        // No public health endpoint; treat a valid (refreshable) token as healthy.
        Ok(self.session.access_token().await.is_ok())
    }
}
