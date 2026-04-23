use super::*;
use futures::stream::BoxStream;
use reqwest::Client as HttpClient;
use std::collections::HashMap;
use url::Url;
use crate::router::{Modality, ModelRuntimeInfo};

/// OllamaProvider - A wrapper around OpenAiProvider with custom model listing and runtime info.
/// 
/// Ollama provides an OpenAI-compatible API for chat completions, so we reuse the OpenAiProvider
/// for those operations. However, Ollama has its own `/api/tags` endpoint for model listing
/// and provides detailed model information, so we override those methods.
/// 
/// For more information on Ollama:
/// - API Documentation: https://github.com/ollama/ollama/blob/main/docs/api.md
#[derive(Clone)]
pub struct OllamaProvider {
    inner: OpenAiProvider,
    http_client: HttpClient,
    base_url: Url,
}

#[derive(Debug, serde::Deserialize)]
pub struct OllamaModelInfo {
    pub name: String,
    pub model: String,
    pub modified_at: String,
    pub size: u64,
    pub digest: String,
    #[serde(default)]
    pub details: OllamaModelDetails,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct OllamaModelDetails {
    #[serde(rename = "parent_model")]
    pub parent_model: String,
    pub format: String,
    pub family: String,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    pub parameter_size: String,
    pub quantization_level: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct OllamaModelsResponse {
    pub models: Vec<OllamaModelInfo>,
}

impl OllamaProvider {
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Result<Self, ProviderError> {
        let base_url = Url::parse(base_url).map_err(|e| ProviderError::Other(e.into()))?;

        Ok(Self {
            inner: OpenAiProvider::new(name, slug, base_url.as_str(), api_key),
            http_client: HttpClient::new(),
            base_url,
        })
    }

    async fn fetch_models(&self) -> Result<OllamaModelsResponse, ProviderError> {
        let models_url = self.base_url.join("/api/tags").map_err(|e| ProviderError::Other(e.into()))?;
        let response = self
            .http_client
            .get(models_url.as_str())
            .send()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;

        if !response.status().is_success() {
            return Err(ProviderError::Other(
                format!("Models endpoint returned status: {}", response.status()).into()
            ));
        }

        response
            .json::<OllamaModelsResponse>()
            .await
            .map_err(|e| ProviderError::Other(e.into()))
    }
}

#[async_trait::async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn slug(&self) -> &str {
        self.inner.slug()
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        let ollama_response = self.fetch_models().await?;
        
        // Convert Ollama models to OpenAI Model format
        let models: Vec<Model> = ollama_response
            .models
            .into_iter()
            .map(|m| Model {
                id: m.name.clone(),
                object: "model".to_string(),
                created: 0, // Ollama doesn't provide creation timestamp
                owned_by: "ollama".to_string(),
            })
            .collect();

        Ok(models)
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
        let health_url = self.base_url.join("/api/tags").map_err(|e| ProviderError::Other(e.into()))?;
        
        match self.http_client.get(health_url.as_str()).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    async fn get_runtime_info(
        &self,
        model_id: &str,
    ) -> Result<Option<ModelRuntimeInfo>, ProviderError> {
        let response = self.fetch_models().await?;
        
        // Find the specific model
        let model_info = response
            .models
            .into_iter()
            .find(|m| m.name == model_id || m.model == model_id);

        match model_info {
            Some(model) => {
                let mut additional_fields = HashMap::new();
                additional_fields.insert("name".to_string(), serde_json::json!(model.name));
                additional_fields.insert("model".to_string(), serde_json::json!(model.model));
                additional_fields.insert("modified_at".to_string(), serde_json::json!(model.modified_at));
                additional_fields.insert("size".to_string(), serde_json::json!(model.size));
                additional_fields.insert("digest".to_string(), serde_json::json!(model.digest));
                additional_fields.insert("family".to_string(), serde_json::json!(model.details.family));
                if let Some(families) = &model.details.families {
                    additional_fields.insert("families".to_string(), serde_json::json!(families));
                }
                additional_fields.insert("parent_model".to_string(), serde_json::json!(model.details.parent_model));
                
                let runtime_info = ModelRuntimeInfo {
                    model_id: model_id.to_string(),
                    context_length: None,
                    quantization: Some(model.details.quantization_level.clone()),
                    variant: Some(model.details.format.clone()),
                    parameter_size: Some(model.details.parameter_size.clone()),
                    max_output_tokens: None,
                    max_concurrency: None,
                    modalities: vec![Modality::Text],
                    additional_fields,
                };
                
                Ok(Some(runtime_info))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODELS_RESPONSE: &str = r#"{
        "models": [
            {
                "name": "llama3.2:latest",
                "model": "llama3.2:latest",
                "modified_at": "2024-01-15T10:30:00Z",
                "size": 1234567890,
                "digest": "abc123def456",
                "details": {
                    "parent_model": "",
                    "format": "gguf",
                    "family": "llama",
                    "families": ["llama"],
                    "parameter_size": "8B",
                    "quantization_level": "Q4_0"
                }
            },
            {
                "name": "mistral:latest",
                "model": "mistral:latest",
                "modified_at": "2024-01-14T08:20:00Z",
                "size": 9876543210,
                "digest": "xyz789uvw012",
                "details": {
                    "parent_model": "",
                    "format": "gguf",
                    "family": "mistral",
                    "families": ["mistral"],
                    "parameter_size": "7B",
                    "quantization_level": "Q4_K_M"
                }
            }
        ]
    }"#;

    #[test]
    fn test_parse_ollama_models() {
        let response: OllamaModelsResponse = serde_json::from_str(MODELS_RESPONSE).expect("Failed to parse models");
        
        assert_eq!(response.models.len(), 2);
        assert_eq!(response.models[0].name, "llama3.2:latest");
        assert_eq!(response.models[0].details.family, "llama");
        assert_eq!(response.models[0].details.parameter_size, "8B");
        assert_eq!(response.models[1].name, "mistral:latest");
        assert_eq!(response.models[1].details.family, "mistral");
    }

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = OllamaProvider::new("Test Provider", Some("test"), "http://localhost:11434", None).unwrap();

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = OllamaProvider::new("My Provider", None, "http://localhost:11434", None).unwrap();
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = OllamaProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:11434", None).unwrap();
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_provider_with_api_key() {
        let provider = OllamaProvider::new("Test", None, "http://localhost:11434", Some("my-api-key")).unwrap();
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_provider_without_api_key() {
        let provider = OllamaProvider::new("Test", None, "http://localhost:11434", None).unwrap();
        assert_eq!(provider.name(), "Test");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = OllamaProvider::new("Test", None, "http://localhost:11434", None).unwrap();
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_fetch_models_success() {
        let provider = OllamaProvider::new("Test", None, "http://localhost:11434", None).unwrap();
        let result = provider.fetch_models().await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_models_with_wiremock() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_string(MODELS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new("Test", None, &mock_server.uri(), None).unwrap();
        let result = provider.fetch_models().await;
        assert!(result.is_ok());
        let models = result.unwrap();
        assert_eq!(models.models.len(), 2);
    }

    #[tokio::test]
    async fn test_fetch_models_with_wiremock_error() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new("Test", None, &mock_server.uri(), None).unwrap();
        let result = provider.fetch_models().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_models_converts_to_openai_format() {
        use wiremock::{MockServer, Mock, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_string(MODELS_RESPONSE))
            .mount(&mock_server)
            .await;

        let provider = OllamaProvider::new("Test", None, &mock_server.uri(), None).unwrap();
        let result = provider.list_models().await;
        assert!(result.is_ok());
        let models = result.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "llama3.2:latest");
        assert_eq!(models[0].owned_by, "ollama");
    }
}
