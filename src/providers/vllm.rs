use super::*;
use async_openai::types::responses::{CreateResponse, Response as ApiResponse};
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// TTL for the cached `/v1/models` response. `get_runtime_info` is called per
/// model by admin model-sync (and historically per request by the router), so
/// without a short-lived cache every model in a sync fans out into an
/// identical GET. 30s keeps data fresh for manual syncs while collapsing the
/// fan-out.
const MODELS_TTL: Duration = Duration::from_secs(30);

/// VllmProvider - A wrapper around OpenAiProvider with vLLM-specific runtime info.
///
/// vLLM provides an OpenAI-compatible API but does NOT support
/// `GET /v1/models/{model_id}` (the retrieve endpoint). Instead we fetch
/// `GET /v1/models` and find the matching entry.
///
/// For more information on vLLM:
/// - API Documentation: https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html
#[derive(Clone)]
pub struct VllmProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
    /// Short-TTL cache of the `/v1/models` list (see `MODELS_TTL`). `None`
    /// inside is a negative cache entry (last fetch failed / non-success).
    models_cache: Arc<RwLock<Option<(Option<ModelListResponse>, Instant)>>>,
}

impl VllmProvider {
    /// Returns `None` if `base_url` is not a valid URL. Callers skip the
    /// provider rather than panicking, so a typo'd base URL in the admin UI
    /// can't 500 the config-reload / provider-create request.
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Option<Self> {
        let base_url = Url::parse(base_url).ok()?;

        Some(Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: streaming_http_client(),
            base_url,
            models_cache: Arc::new(RwLock::new(None)),
        })
    }

    /// Fetch `GET /v1/models` through the short-TTL cache.
    ///
    /// Returns `Ok(Some(body))` on success, `Ok(None)` on a non-success HTTP
    /// status (cached negatively so we don't hammer a broken endpoint), and
    /// `Err` on a network failure (not cached, so a recovered endpoint is
    /// picked up immediately).
    async fn fetch_models(&self) -> Result<Option<ModelListResponse>, ProviderError> {
        {
            let guard = self.models_cache.read().await;
            if let Some((ref cached, cached_at)) = *guard {
                if cached_at.elapsed() < MODELS_TTL {
                    return Ok(cached.clone());
                }
            }
        }

        let models_url = self
            .base_url
            .join("v1/models")
            .map_err(|e| ProviderError::Other(e.into()))?;

        let response = self
            .http_client
            .get(models_url.as_str())
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        let result = if response.status().is_success() {
            let body: ModelListResponse = response
                .json()
                .await
                .map_err(|e| ProviderError::Other(e.into()))?;
            Some(body)
        } else {
            tracing::debug!(
                provider = self.name(),
                status = %response.status(),
                "vLLM /v1/models request failed"
            );
            None
        };

        *self.models_cache.write().await = Some((result.clone(), Instant::now()));
        Ok(result)
    }
}

#[async_trait::async_trait]
impl Provider for VllmProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        self.inner.list_models().await
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        self.inner.chat_completions(request).await
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        self.inner.chat_completions_stream(request)
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        self.inner.health_check().await
    }

    async fn get_runtime_info(&self, model_id: &str) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        let body = match self.fetch_models().await {
            Ok(body) => body,
            Err(e) => {
                tracing::debug!(
                    provider = self.name(),
                    error = %e,
                    "vLLM /v1/models request failed"
                );
                return Err(e);
            }
        };

        let Some(body) = body else {
            return Ok(None);
        };

        // Find the model in the list
        let model_entry = body.data.iter().find(|m| m.id == model_id);

        // Model not listed by vLLM (typo, not loaded, or removed). Return
        // `None` — the same signal `OpenAiProvider` gives for a 404 — rather
        // than a half-empty info struct. The router's presence-gate treats a
        // `None` result as "not cached", so a model that appears later (e.g.
        // hot-loaded) is picked up without a restart.
        let Some(model_entry) = model_entry else {
            return Ok(None);
        };

        // vLLM reports its context window as `max_model_len` on the model
        // entry (not the `context_length` key some other backends use); the
        // `ModelListEntry.context_length` field is always None for vLLM, so
        // fall back to the raw extra field.
        let context_length = model_entry.context_length
            .or_else(|| {
                model_entry
                    .extra
                    .get("max_model_len")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
            });
        let max_concurrency = model_entry.max_concurrency;

        let mut additional_fields = model_entry.extra.clone();
        // id is already stored as model_id, drop it from extras
        additional_fields.remove("id");

        let runtime_info = ModelRuntimeInfo {
            model_id: model_id.to_string(),
            context_length,
            quantization: None,
            variant: None,
            parameter_size: None,
            max_output_tokens: None,
            max_concurrency,
            modalities: vec![Modality::Text],
            additional_fields,
        };

        Ok(Some(runtime_info))
    }

    async fn responses(
        &self,
        request: &CreateResponse,
    ) -> Result<ApiResponse, ProviderError> {
        self.inner.responses(request).await
    }
}

#[cfg(test)]
mod tests {
use super::*;
use futures::stream::BoxStream;
use crate::router::ModelRuntimeInfo;

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = VllmProvider::new("Test Provider", Some("test"), "http://localhost:8080", Some("test-key")).unwrap();

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = VllmProvider::new("My Provider", None, "http://localhost:8080", Some("key")).unwrap();
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = VllmProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:8080", Some("key")).unwrap();
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", Some("my-api-key")).unwrap();
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", None).unwrap();
        assert_eq!(provider.name(), "Test");
    }

    #[test]
    fn test_new_with_invalid_url_returns_none() {
        // A malformed base URL must not panic (the factory runs inside API
        // handlers / config reload); it should just skip the provider.
        assert!(VllmProvider::new("Test", None, "not a valid url", None).is_none());
        assert!(VllmProvider::new("Test", None, "http://", None).is_none());
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = VllmProvider::new("Test", None, "http://localhost:8080", Some("key")).unwrap();
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_list_models_error_handling() {
        // Valid URL, unreachable port: exercises the network-error path.
        let provider = VllmProvider::new("Test", None, "http://127.0.0.1:1", Some("key")).unwrap();
        let result = provider.list_models().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_completions_stream_error_handling() {
        let provider = VllmProvider::new("Test", None, "http://127.0.0.1:1", Some("key")).unwrap();
        let request = CreateChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![],
            ..Default::default()
        };
        let result = provider.chat_completions_stream(&request);
        assert!(result.is_ok());
    }

    const VLLM_MODELS_RESPONSE: &str = r#"{
        "object": "list",
        "data": [
            {
                "id": "qwen3.8-27b-fp8",
                "object": "model",
                "created": 1786742535,
                "owned_by": "vllm",
                "root": "Qwen/Qwen3.8-27B-FP8",
                "parent": null,
                "max_model_len": 256000,
                "permission": []
            }
        ]
    }"#;

    #[tokio::test]
    async fn test_get_runtime_info_maps_max_model_len_to_context_length() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(VLLM_MODELS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = VllmProvider::new("Test", None, &mock_server.uri(), Some("key")).unwrap();
        let info = provider
            .get_runtime_info("qwen3.8-27b-fp8")
            .await
            .expect("fetch ok")
            .expect("model found");

        // vLLM only reports max_model_len; the provider must map it.
        assert_eq!(info.context_length(), Some(256000));
        assert_eq!(info.max_concurrency(), None);
        // root/parent ride along in additional_fields for display
        assert_eq!(
            info.additional_fields.get("root").and_then(|v| v.as_str()),
            Some("Qwen/Qwen3.8-27B-FP8")
        );
    }

    #[tokio::test]
    async fn test_get_runtime_info_unknown_model_returns_none() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(VLLM_MODELS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = VllmProvider::new("Test", None, &mock_server.uri(), Some("key")).unwrap();
        let info = provider.get_runtime_info("does-not-exist").await;
        assert!(info.is_ok());
        assert!(info.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_runtime_info_non_success_returns_none() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let provider = VllmProvider::new("Test", None, &mock_server.uri(), Some("key")).unwrap();
        let info = provider.get_runtime_info("qwen3.8-27b-fp8").await;
        // Non-success HTTP status is "no info", not an error (parity with the
        // pre-refactor behaviour).
        assert!(info.is_ok());
        assert!(info.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_runtime_info_caches_model_list() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string(VLLM_MODELS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = VllmProvider::new("Test", None, &mock_server.uri(), Some("key")).unwrap();
        // Two calls for two models must not refetch (admin model-sync fan-out).
        provider.get_runtime_info("qwen3.8-27b-fp8").await.unwrap();
        provider.get_runtime_info("qwen3.8-27b-fp8").await.unwrap();

        let requests = mock_server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.url.path() == "/v1/models")
            .count();
        assert_eq!(requests, 1, "expected the model list to be fetched once, then served from cache");
    }
}
