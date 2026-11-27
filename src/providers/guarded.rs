use std::collections::HashMap;
use std::sync::Arc;

use async_openai::types::models::Model;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;

use crate::providers::audio::{SpeechRequest, SpeechResponse, TranscriptionRequest, TranscriptionResponse};
use crate::providers::decision::{DecisionRequest, DecisionResponse};
use crate::providers::egress::{caller, Egress};
use crate::providers::image::{ImageGenerationRequest, ImageResponse, ImageUploadRequest};
use crate::providers::{ChatRequest, Provider, ProviderError, StreamingChunk};
use crate::router::ModelRuntimeInfo;

pub type Access = Arc<std::sync::RwLock<HashMap<String, (Option<i64>, String)>>>;

pub const NOT_APPROVED: &str = "provider is not approved yet: it is only contacted when its owner checks or verifies it";

pub struct GuardedProvider {
    inner: Arc<dyn Provider>,
    base_url: String,
    slug: String,
    owner: Option<i64>,
    access: Access,
    egress: Egress,
}

impl GuardedProvider {
    pub fn wrap(inner: Arc<dyn Provider>, record: &crate::db::Provider, egress: &Egress, access: Access) -> Arc<dyn Provider> {
        Arc::new(Self {
            inner,
            base_url: record.base_url.clone(),
            slug: record.slug.clone(),
            owner: record.owner_user_id,
            access,
            egress: egress.clone(),
        })
    }

    fn approved(&self) -> bool {
        self.access
            .read()
            .map(|a| a.get(&self.slug).is_some_and(|(_, status)| status == crate::db::APPROVED))
            .unwrap_or(false)
    }

    pub fn is_held(access: &Access, slug: &str) -> bool {
        access
            .read()
            .map(|a| a.get(slug).is_some_and(|(owner, status)| owner.is_some() && status != crate::db::APPROVED))
            .unwrap_or(true)
    }

    fn permitted(&self) -> Result<(), ProviderError> {
        if self.approved() {
            return Ok(());
        }
        match caller() {
            Some(c) if c.is_admin || Some(c.user_id) == self.owner => Ok(()),
            _ => Err(ProviderError::Unsupported(NOT_APPROVED.to_string())),
        }
    }

    async fn allow(&self) -> Result<(), ProviderError> {
        self.permitted()?;
        let (_, host, _) =
            crate::ssrf::parse_public_url(&self.base_url).map_err(|e| ProviderError::Other(e.to_string().into()))?;
        if let Err(e) = self.egress.pinned().checked(&host).await {
            tracing::warn!(provider = %self.slug, host = %host, reason = ?e, "User provider refused by the egress guard");
            return Err(ProviderError::Other(e.to_string().into()));
        }
        Ok(())
    }
}

fn phrase(text: &str) -> String {
    match crate::providers::attribution::request_error_phrase(text) {
        Some(p) => format!(": {p}"),
        None => String::new(),
    }
}

pub fn sanitize(error: ProviderError) -> ProviderError {
    let connection = error.is_connection_failure();
    match error {
        ProviderError::RateLimit { retry_after_ms, .. } => {
            ProviderError::RateLimit { retry_after_ms, message: "upstream rate limited the request".to_string() }
        }
        ProviderError::ServerError { message, status_code } => ProviderError::ServerError {
            message: match status_code {
                Some(code) => format!("upstream returned HTTP {code}{}", phrase(&message)),
                None => format!("upstream request failed{}", phrase(&message)),
            },
            status_code,
        },
        ProviderError::Timeout => ProviderError::Timeout,
        ProviderError::Authentication(_) => ProviderError::Authentication("upstream rejected the credentials".to_string()),
        ProviderError::NotFound(_) => ProviderError::NotFound("upstream returned not found".to_string()),
        ProviderError::Unsupported(_) => ProviderError::Unsupported("upstream does not support this request".to_string()),
        other if connection => {
            let _ = other;
            ProviderError::Other("error sending request: connection failed".to_string().into())
        }
        ProviderError::OpenAIError(e) => {
            let text = e.to_string();
            ProviderError::Other(format!("upstream returned an unusable response{}", phrase(&text)).into())
        }
        ProviderError::Other(e) => {
            let text = e.to_string();
            ProviderError::Other(format!("upstream request failed{}", phrase(&text)).into())
        }
    }
}

#[async_trait]
impl Provider for GuardedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.allow().await?;
        self.inner.list_models().await.map_err(sanitize)
    }

    async fn chat_completions(&self, request: &ChatRequest) -> Result<crate::providers::ChatResponse, ProviderError> {
        self.allow().await?;
        self.inner.chat_completions(request).await.map_err(sanitize)
    }

    fn chat_completions_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        self.permitted()?;
        let inner = self.inner.clone();
        let base_url = self.base_url.clone();
        let egress = self.egress.clone();
        let request = request.clone();
        let stream = async_stream::stream! {
            let host = match crate::ssrf::parse_public_url(&base_url) {
                Ok((_, host, _)) => host,
                Err(e) => {
                    yield Err(ProviderError::Other(e.to_string().into()));
                    return;
                }
            };
            if let Err(e) = egress.pinned().checked(&host).await {
                yield Err(ProviderError::Other(e.to_string().into()));
                return;
            }
            match inner.chat_completions_stream(&request) {
                Ok(mut upstream) => {
                    while let Some(item) = upstream.next().await {
                        yield item.map_err(sanitize);
                    }
                }
                Err(e) => yield Err(sanitize(e)),
            }
        };
        Ok(stream.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        self.allow().await?;
        self.inner.health_check().await.map_err(sanitize)
    }

    async fn responses(&self, request: &CreateResponse) -> Result<ApiResponse, ProviderError> {
        self.allow().await?;
        self.inner.responses(request).await.map_err(sanitize)
    }

    async fn responses_with_body(
        &self,
        request: &CreateResponse,
        body_defaults: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<ApiResponse, ProviderError> {
        self.allow().await?;
        self.inner.responses_with_body(request, body_defaults).await.map_err(sanitize)
    }

    async fn transcriptions(&self, request: &TranscriptionRequest) -> Result<TranscriptionResponse, ProviderError> {
        self.allow().await?;
        self.inner.transcriptions(request).await.map_err(sanitize)
    }

    async fn speech(&self, request: &SpeechRequest) -> Result<SpeechResponse, ProviderError> {
        self.allow().await?;
        self.inner.speech(request).await.map_err(sanitize)
    }

    async fn list_voices(&self, model: &str) -> Result<Option<Vec<String>>, ProviderError> {
        self.allow().await?;
        self.inner.list_voices(model).await.map_err(sanitize)
    }

    async fn image_generations(&self, request: &ImageGenerationRequest) -> Result<ImageResponse, ProviderError> {
        self.allow().await?;
        self.inner.image_generations(request).await.map_err(sanitize)
    }

    async fn image_uploads(&self, request: &ImageUploadRequest) -> Result<ImageResponse, ProviderError> {
        self.allow().await?;
        self.inner.image_uploads(request).await.map_err(sanitize)
    }

    async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
        self.allow().await?;
        self.inner.decide(request).await.map_err(sanitize)
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        self.allow().await?;
        self.inner.get_runtime_info(model_id).await.map_err(sanitize)
    }

    fn reports_modalities(&self) -> bool {
        self.inner.reports_modalities()
    }
}

#[cfg(test)]
#[path = "guarded_tests.rs"]
mod tests;
