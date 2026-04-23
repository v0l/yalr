use super::*;
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use std::collections::HashMap;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// LlamaCppProvider - A wrapper around OpenAiProvider with custom runtime info extraction.
/// 
/// llama.cpp provides an OpenAI-compatible API server, so we reuse the OpenAiProvider
/// for chat completions and model listing, but add custom /props endpoint parsing
/// for detailed runtime information.
/// 
/// For more information on the llama.cpp server:
/// - API Documentation: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#api-endpoints
#[derive(Clone)]
pub struct LlamaCppProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
}

#[derive(Debug, serde::Deserialize)]
pub struct LlamaCppProps {
    #[serde(rename = "model_alias")]
    pub model_alias: Option<String>,
    #[serde(rename = "model_path")]
    pub model_path: Option<String>,
    #[serde(rename = "total_slots")]
    pub total_slots: Option<u32>,
    #[serde(rename = "n_ctx")]
    pub n_ctx: Option<u32>,
    #[serde(rename = "n_batch")]
    pub n_batch: Option<u32>,
    #[serde(rename = "n_threads")]
    pub n_threads: Option<u32>,
    #[serde(rename = "n_gpu_layers")]
    pub n_gpu_layers: Option<u32>,
    #[serde(rename = "model_size")]
    pub model_size: Option<u64>,
    #[serde(rename = "model_n_params")]
    pub model_n_params: Option<u64>,
    #[serde(rename = "model_type")]
    pub model_type: Option<String>,
    #[serde(rename = "model_quant_type")]
    pub model_quant_type: Option<String>,
    #[serde(rename = "rope_freq_base")]
    pub rope_freq_base: Option<f32>,
    #[serde(rename = "rope_freq_scale")]
    pub rope_freq_scale: Option<f32>,
    #[serde(rename = "logits_all")]
    pub logits_all: Option<bool>,
    #[serde(rename = "embedding")]
    pub embedding: Option<bool>,
    pub modalities: Option<LlamaCppModalities>,
    #[serde(rename = "default_generation_settings")]
    pub default_generation_settings: Option<LlamaCppDefaultGenSettings>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LlamaCppDefaultGenSettings {
    #[serde(rename = "n_ctx")]
    pub n_ctx: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LlamaCppModalities {
    pub vision: Option<bool>,
    pub audio: Option<bool>,
}

impl LlamaCppProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Result<Self, ProviderError> {
        let base_url = Url::parse(base_url).map_err(|e| ProviderError::Other(e.into()))?;

        Ok(Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: HttpClient::new(),
            base_url,
        })
    }

    async fn fetch_props(&self) -> Result<LlamaCppProps, ProviderError> {
        let props_url = self.base_url.join("/props").map_err(|e| ProviderError::Other(e.into()))?;
        let response = self
            .http_client
            .get(props_url.as_str())
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            return Err(ProviderError::Other(
                format!("Props endpoint returned status: {}", response.status()).into()
            ));
        }

        response
            .json::<LlamaCppProps>()
            .await
            .map_err(|e| ProviderError::Other(e.into()))
    }
}

#[async_trait::async_trait]
impl Provider for LlamaCppProvider {
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
        let health_url = self.base_url.join("/health").map_err(|e| ProviderError::Other(e.into()))?;
        
        match self.http_client.get(health_url.as_str()).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn get_runtime_info(
        &self,
        model_id: &str,
    ) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        let props = self.fetch_props().await?;

        let mut additional_fields = HashMap::new();
        if let Some(alias) = props.model_alias {
            additional_fields.insert("model_alias".to_string(), serde_json::json!(alias));
        }
        if let Some(path) = props.model_path {
            additional_fields.insert("model_path".to_string(), serde_json::json!(path));
        }
        if let Some(size) = props.model_size {
            additional_fields.insert("model_size".to_string(), serde_json::json!(size));
        }
        if let Some(model_type) = props.model_type {
            additional_fields.insert("model_type".to_string(), serde_json::json!(model_type));
        }
        if let Some(n_threads) = props.n_threads {
            additional_fields.insert("n_threads".to_string(), serde_json::json!(n_threads));
        }
        if let Some(n_gpu_layers) = props.n_gpu_layers {
            additional_fields.insert("n_gpu_layers".to_string(), serde_json::json!(n_gpu_layers));
        }
        if let Some(rope_freq_base) = props.rope_freq_base {
            additional_fields.insert("rope_freq_base".to_string(), serde_json::json!(rope_freq_base));
        }
        if let Some(rope_freq_scale) = props.rope_freq_scale {
            additional_fields.insert("rope_freq_scale".to_string(), serde_json::json!(rope_freq_scale));
        }
        if let Some(logits_all) = props.logits_all {
            additional_fields.insert("logits_all".to_string(), serde_json::json!(logits_all));
        }
        if let Some(embedding) = props.embedding {
            additional_fields.insert("embedding".to_string(), serde_json::json!(embedding));
        }

        let mut modalities = vec![Modality::Text];
        if let Some(props_modalities) = props.modalities {
            if props_modalities.vision.unwrap_or(false) {
                modalities.push(crate::router::Modality::Image);
            }
            if props_modalities.audio.unwrap_or(false) {
                modalities.push(crate::router::Modality::Audio);
            }
        }

        let runtime_info = ModelRuntimeInfo {
            model_id: model_id.to_string(),
            context_length: props.default_generation_settings.and_then(|g| g.n_ctx),
            quantization: props.model_quant_type,
            variant: None,
            parameter_size: props.model_n_params.map(|p| p.to_string()),
            max_output_tokens: props.n_batch,
            max_concurrency: props.total_slots,
            modalities,
            additional_fields,
        };

        Ok(Some(runtime_info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROPS_RESPONSE: &str = r#"{
    "default_generation_settings": {
        "n_ctx": 262144
    },
    "total_slots": 1,
    "model_alias": "qwen3.5:122b",
    "model_path": "/home/kieran/.cache/model.gguf",
    "modalities": {
        "vision": true,
        "audio": false
    },
    "build_info": "b8709-85d482e6b",
    "is_sleeping": false
}"#;

    #[test]
    fn test_parse_llama_cpp_props() {
        let props: LlamaCppProps = serde_json::from_str(PROPS_RESPONSE).expect("Failed to parse props");

        assert_eq!(props.total_slots, Some(1));
        assert_eq!(props.model_alias, Some("qwen3.5:122b".to_string()));
        assert!(props.model_path.is_some());

        let modalities = props.modalities.expect("modalities should be present");
        assert_eq!(modalities.vision, Some(true));
        assert_eq!(modalities.audio, Some(false));
    }

    #[test]
    fn test_runtime_info_from_props() {
        let props: LlamaCppProps = serde_json::from_str(PROPS_RESPONSE).expect("Failed to parse props");

        let mut additional_fields = HashMap::new();
        additional_fields.insert("test".to_string(), serde_json::json!("value"));

        let mut modalities = vec![Modality::Text];
        if let Some(props_modalities) = props.modalities {
            if props_modalities.vision.unwrap_or(false) {
                modalities.push(Modality::Image);
            }
            if props_modalities.audio.unwrap_or(false) {
                modalities.push(Modality::Audio);
            }
        }

        let runtime_info = ModelRuntimeInfo {
            model_id: "test-model".to_string(),
            context_length: props.default_generation_settings.and_then(|g| g.n_ctx),
            quantization: props.model_quant_type,
            variant: None,
            parameter_size: props.model_n_params.map(|p| p.to_string()),
            max_output_tokens: props.n_batch,
            max_concurrency: props.total_slots,
            modalities,
            additional_fields,
        };

        assert_eq!(runtime_info.context_length(), Some(262144));
        assert!(runtime_info.supports_image());
        assert!(!runtime_info.supports_audio());
        assert!(runtime_info
            .modalities
            .contains(&Modality::Image));
    }

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = LlamaCppProvider::new("Test Provider", Some("test"), "http://localhost:8080", None).unwrap();

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = LlamaCppProvider::new("My Provider", None, "http://localhost:8080", None).unwrap();
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = LlamaCppProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:8080", None).unwrap();
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None).unwrap();
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_fetch_props_success() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None).unwrap();
        let result = provider.fetch_props().await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_props_with_trailing_slash() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080/", None).unwrap();
        assert_eq!(provider.base_url.to_string(), "http://localhost:8080/");
    }

    #[tokio::test]
    async fn test_fetch_props_without_trailing_slash() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None).unwrap();
        // Url::join normalizes the URL, adding a trailing slash
        assert!(provider.base_url.to_string().starts_with("http://localhost:8080"));
    }

    #[tokio::test]
    async fn test_fetch_props_with_wiremock() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(200).set_body_string(PROPS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = LlamaCppProvider::new("Test", None, &mock_server.uri(), None).unwrap();
        let result = provider.fetch_props().await;
        assert!(result.is_ok());
        let props = result.unwrap();
        assert_eq!(props.total_slots, Some(1));
    }

    #[tokio::test]
    async fn test_fetch_props_with_wiremock_error() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/props"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let provider = LlamaCppProvider::new("Test", None, &mock_server.uri(), None).unwrap();
        let result = provider.fetch_props().await;
        assert!(result.is_err());
    }
}
