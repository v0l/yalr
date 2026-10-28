use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use base64::Engine;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::api::shared::{error_body, HandlerError};
use crate::auth::admin::AuthenticatedUser;
use crate::payments::biller::{BillingError, TokenUsage};
use crate::payments::guard::{insufficient_funds_response, BillingGuard};
use crate::request_log::recorder::{Inbound, Pending};
use crate::request_log::{redact, settings, Trace};
use crate::state::AppState;
use crate::ProviderError;

#[derive(Serialize)]
struct BinaryBody<'a> {
    content_type: &'a str,
    bytes: usize,
    base64: String,
}

pub fn binary_envelope(content_type: &str, data: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&BinaryBody {
        content_type,
        bytes: data.len(),
        base64: base64::engine::general_purpose::STANDARD.encode(data),
    })
    .unwrap_or_default()
}

#[derive(Serialize)]
pub struct UploadedFile<'a> {
    pub field: &'a str,
    pub file_name: &'a str,
    pub content_type: Option<&'a str>,
    pub bytes: usize,
    pub base64: String,
}

impl<'a> UploadedFile<'a> {
    pub fn new(field: &'a str, file_name: &'a str, content_type: Option<&'a str>, data: &[u8]) -> Self {
        Self {
            field,
            file_name,
            content_type,
            bytes: data.len(),
            base64: base64::engine::general_purpose::STANDARD.encode(data),
        }
    }
}

#[derive(Serialize)]
pub struct MultipartBody<'a> {
    pub fields: Vec<(&'a str, &'a str)>,
    pub files: Vec<UploadedFile<'a>>,
}

pub fn response_body(content_type: &str, data: &[u8]) -> Vec<u8> {
    let is_json = content_type.split(';').next().is_some_and(|t| t.trim().ends_with("json"));
    let is_text = content_type.starts_with("text/");
    if is_json || is_text {
        data.to_vec()
    } else {
        binary_envelope(content_type, data)
    }
}

#[derive(Deserialize)]
struct ReportedUsage {
    #[serde(default, alias = "prompt_tokens")]
    input_tokens: u32,
    #[serde(default, alias = "completion_tokens")]
    output_tokens: u32,
    #[serde(default, alias = "prompt_tokens_details")]
    input_tokens_details: Option<CachedDetails>,
}

#[derive(Deserialize)]
struct CachedDetails {
    #[serde(default)]
    cached_tokens: u32,
}

#[derive(Deserialize)]
struct WithUsage {
    usage: Option<ReportedUsage>,
}

pub fn usage_in_json(body: &[u8]) -> Option<TokenUsage> {
    let usage = serde_json::from_slice::<WithUsage>(body).ok()?.usage?;
    Some(TokenUsage {
        prompt_tokens: usage.input_tokens,
        completion_tokens: usage.output_tokens,
        cached_tokens: usage.input_tokens_details.map_or(0, |d| d.cached_tokens),
        cache_write_tokens: 0,
    })
}

pub fn usage_in_sse(body: &[u8]) -> Option<TokenUsage> {
    String::from_utf8_lossy(body)
        .lines()
        .rev()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|data| usage_in_json(data.trim().as_bytes()))
}

pub fn usage_in_body(content_type: &str, body: &[u8]) -> Option<TokenUsage> {
    if content_type.starts_with("text/event-stream") {
        usage_in_sse(body)
    } else {
        usage_in_json(body)
    }
}

fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .or_else(|| peer.map(|p| p.ip().to_string()))
}

pub struct RequestInfo<'a> {
    pub endpoint: &'static str,
    pub model: &'a str,
    pub stream: bool,
    pub headers: &'a HeaderMap,
    pub peer: Option<SocketAddr>,
}

pub struct Metered {
    state: Arc<AppState>,
    authenticated: AuthenticatedUser,
    model: String,
    log: Option<Pending>,
    trace: Trace,
    billing: Option<BillingGuard>,
}

fn billing_error(e: BillingError) -> HandlerError {
    match e {
        BillingError::InsufficientFunds { required, available } => {
            let (code, json) = insufficient_funds_response(required, available);
            (code, json.0.to_string())
        }
        e => (StatusCode::INTERNAL_SERVER_ERROR, error_body(&format!("Billing error: {e}"), "billing_error")),
    }
}

impl Metered {
    pub fn begin(
        state: &Arc<AppState>,
        authenticated: &AuthenticatedUser,
        info: RequestInfo<'_>,
        body: impl FnOnce() -> Vec<u8>,
    ) -> Self {
        let trace = Trace::new();
        let settings = settings::get();
        let log = settings.enabled.then(|| {
            let user = &authenticated.user;
            let inbound = Inbound {
                request_id: uuid::Uuid::new_v4().to_string(),
                endpoint: info.endpoint,
                stream: info.stream,
                model: info.model.to_string(),
                user_id: Some(user.id),
                user_name: user.username.clone(),
                api_key_id: authenticated.api_key.as_ref().map(|k| k.id),
                api_key_name: authenticated.api_key.as_ref().map(|k| k.name.clone()),
                client_ip: client_ip(info.headers, info.peer),
                client_headers: redact::axum_headers(info.headers),
                body: if settings.store_bodies { body() } else { Vec::new() },
            };
            Pending::new(state.db.log_writer.clone(), inbound, trace.clone())
        });
        Self {
            state: state.clone(),
            authenticated: authenticated.clone(),
            model: info.model.to_string(),
            log,
            trace,
            billing: None,
        }
    }

    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    pub fn reject(mut self, error: HandlerError) -> HandlerError {
        let status = if error.0 == StatusCode::PAYMENT_REQUIRED || error.0 == StatusCode::FORBIDDEN {
            "rejected"
        } else {
            "error"
        };
        if let Some(pending) = self.log.take() {
            pending.complete(status, Some(error.1.clone()));
        }
        error
    }

    pub async fn admit(&mut self) -> Result<(), HandlerError> {
        match BillingGuard::try_create(&self.state, Some(self.authenticated.user.id), &self.model).await {
            Ok(guard) => {
                self.billing = Some(guard);
                Ok(())
            }
            Err(e) => Err(billing_error(e)),
        }
    }

    async fn settle(&self, usage: Option<TokenUsage>) -> i64 {
        let usage = usage.unwrap_or_default();
        let billed_msat = match &self.billing {
            Some(guard) => guard.finalize_usage(usage).await,
            None => 0,
        };
        let api_key_id = self.authenticated.api_key.as_ref().map(|k| k.id);
        crate::usage::record(&self.state, api_key_id, &self.model, usage, billed_msat).await;
        billed_msat
    }

    pub async fn succeed(mut self, response: Vec<u8>, usage: Option<TokenUsage>) {
        let billed_msat = self.settle(usage).await;
        if let Some(mut pending) = self.log.take() {
            if let Some(usage) = usage {
                pending.observe_tokens(usage);
            }
            pending.set_response(response, None);
            pending.set_cost(billed_msat);
            pending.complete("ok", None);
        }
    }

    pub fn fail(mut self, error: &str) {
        if let Some(pending) = self.log.take() {
            pending.complete("error", Some(error.to_string()));
        }
    }

    pub fn tap(
        self,
        content_type: String,
        upstream: BoxStream<'static, Result<Bytes, ProviderError>>,
    ) -> BoxStream<'static, Result<Bytes, ProviderError>> {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let finish = Finish { metered: Some(self), collected: collected.clone(), content_type, failed: None };
        let finish = Arc::new(Mutex::new(finish));
        let on_end = finish.clone();
        let stream = upstream.map(move |chunk| {
            match &chunk {
                Ok(bytes) => {
                    if let Ok(mut buffer) = collected.lock() {
                        buffer.extend_from_slice(bytes);
                    }
                }
                Err(e) => {
                    if let Ok(mut finish) = finish.lock() {
                        finish.failed = Some(e.to_string());
                    }
                }
            }
            chunk
        });
        let done = futures::stream::once(async move {
            let finishing = on_end.lock().ok().and_then(|mut f| f.take());
            if let Some((metered, body, content_type, failed)) = finishing {
                metered.finish_stream(body, &content_type, failed).await;
            }
            None
        });
        stream.map(Some).chain(done).filter_map(futures::future::ready).boxed()
    }

    async fn finish_stream(mut self, body: Vec<u8>, content_type: &str, failed: Option<String>) {
        let usage = usage_in_body(content_type, &body);
        let billed_msat = self.settle(usage).await;
        if let Some(mut pending) = self.log.take() {
            if let Some(usage) = usage {
                pending.observe_tokens(usage);
            }
            pending.set_response(response_body(content_type, &body), None);
            pending.set_cost(billed_msat);
            match failed {
                Some(error) => pending.complete("error", Some(error)),
                None => pending.complete("ok", None),
            }
        }
    }
}

struct Finish {
    metered: Option<Metered>,
    collected: Arc<Mutex<Vec<u8>>>,
    content_type: String,
    failed: Option<String>,
}

impl Finish {
    fn take(&mut self) -> Option<(Metered, Vec<u8>, String, Option<String>)> {
        let metered = self.metered.take()?;
        let body = self.collected.lock().map(|mut b| std::mem::take(&mut *b)).unwrap_or_default();
        Some((metered, body, std::mem::take(&mut self.content_type), self.failed.take()))
    }
}

impl Drop for Finish {
    fn drop(&mut self) {
        if let Some((metered, body, content_type, _)) = self.take() {
            tokio::spawn(async move {
                metered.finish_stream(body, &content_type, Some("client disconnected".to_string())).await;
            });
        }
    }
}

#[cfg(test)]
mod tests;
