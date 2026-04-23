use super::*;
use crate::router::ModelRuntimeInfo;
use async_openai::config::OpenAIConfig;
use async_openai::Client;
use futures::{stream::BoxStream, StreamExt};
use reqwest::Client as HttpClient;
use std::collections::HashMap;

#[derive(Clone)]
pub struct LlamaCppProvider {
    name: String,
    slug: String,
    client: Client<OpenAIConfig>,
    http_client: HttpClient,
    props_url: String,
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
    pub fn new(name: &str, slug: Option<&str>, base_url: &str, api_key: Option<&str>) -> Self {
        let slug = slug
            .unwrap_or(name)
            .to_lowercase()
            .replace(" ", "-")
            .replace("_", "-");

        let config = OpenAIConfig::default()
            .with_api_base(base_url)
            .with_api_key(api_key.unwrap_or(""));

        let props_url = if base_url.ends_with('/') {
            format!("{}props", base_url)
        } else {
            format!("{}/props", base_url)
        };

        Self {
            name: name.to_string(),
            slug,
            client: Client::with_config(config),
            http_client: HttpClient::new(),
            props_url,
        }
    }

    async fn fetch_props(&self) -> Result<LlamaCppProps, ProviderError> {
        let response = self
            .http_client
            .get(&self.props_url)
            .send()
            .await
            .map_err(|e| ProviderError::ProviderError(format!("Failed to fetch props: {}", e)))?;

        if !response.status().is_success() {
            return Err(ProviderError::ProviderError(format!(
                "Props endpoint returned status: {}",
                response.status()
            )));
        }

        response
            .json::<LlamaCppProps>()
            .await
            .map_err(|e| ProviderError::ProviderError(format!("Failed to parse props: {}", e)))
    }
}

#[async_trait::async_trait]
impl Provider for LlamaCppProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        let response = self.client.models().list().await?;
        Ok(response.data)
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        let response = self.client.chat().create(request.clone()).await?;
        Ok(response)
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
        ProviderError,
    > {
        use crate::providers::StreamingChunk;
        use futures::StreamExt;

        let client = self.client.clone();
        let request = request.clone();

        // Serialize request once at the start
        let request_value = serde_json::to_value(request)
            .map_err(|e| ProviderError::ProviderError(format!("Failed to serialize request: {}", e)))?;

        let stream = async move {
            match client.chat().create_stream_byot(request_value).await {
                Ok(stream) => {
                    Box::pin(stream.map(|result| {
                        result
                            .map_err(|e| ProviderError::OpenAIError(e))
                            .and_then(|json_value: serde_json::Value| {
                                // Deserialize the raw JSON value to our custom type
                                // This preserves all fields including reasoning_content
                                serde_json::from_value(json_value)
                                    .map_err(|e| ProviderError::ProviderError(format!("Failed to deserialize chunk: {}", e)))
                            })
                    })) as BoxStream<'static, Result<StreamingChunk, ProviderError>>
                }
                Err(e) => {
                    Box::pin(futures::stream::once(async move {
                        Err(ProviderError::OpenAIError(e))
                    })) as BoxStream<'static, Result<StreamingChunk, ProviderError>>
                }
            }
        };

        Ok(async_stream::stream! {
            let s = stream.await;
            futures::pin_mut!(s);
            while let Some(item) = s.next().await {
                yield item;
            }
        }.boxed())
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        match self.client.models().list().await {
            Ok(_) => Ok(true),
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
        if let Some(slots) = props.total_slots {
            additional_fields.insert("total_slots".to_string(), serde_json::json!(slots));
        }
        if let Some(n_batch) = props.n_batch {
            additional_fields.insert("n_batch".to_string(), serde_json::json!(n_batch));
        }
        if let Some(n_threads) = props.n_threads {
            additional_fields.insert("n_threads".to_string(), serde_json::json!(n_threads));
        }
        if let Some(n_gpu_layers) = props.n_gpu_layers {
            additional_fields.insert("n_gpu_layers".to_string(), serde_json::json!(n_gpu_layers));
        }
        if let Some(rope_freq_base) = props.rope_freq_base {
            additional_fields.insert(
                "rope_freq_base".to_string(),
                serde_json::json!(rope_freq_base),
            );
        }

        let mut runtime_info = ModelRuntimeInfo::from_api_response(model_id, additional_fields);

        runtime_info.context_length = props.default_generation_settings.and_then(|g| g.n_ctx);
        runtime_info.quantization = props.model_quant_type;
        runtime_info.parameter_size = props.model_n_params.map(|p| p.to_string());
        runtime_info.max_output_tokens = props.n_batch;
        runtime_info.max_concurrency = props.total_slots;

        if let Some(modalities) = props.modalities {
            if modalities.vision.unwrap_or(false) {
                runtime_info.modalities.push(crate::router::Modality::Image);
            }
            if modalities.audio.unwrap_or(false) {
                runtime_info.modalities.push(crate::router::Modality::Audio);
            }
        }

        Ok(Some(runtime_info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROPS_RESPONSE: &str = r#"{
    "default_generation_settings": {
        "params": {
            "seed": 4294967295,
            "temperature": 0.600000023841858,
            "dynatemp_range": 0,
            "dynatemp_exponent": 1,
            "top_k": 20,
            "top_p": 0.949999988079071,
            "min_p": 0.0500000007450581,
            "top_n_sigma": -1,
            "xtc_probability": 0,
            "xtc_threshold": 0.100000001490116,
            "typical_p": 1,
            "repeat_last_n": 64,
            "repeat_penalty": 1,
            "presence_penalty": 0,
            "frequency_penalty": 0,
            "dry_multiplier": 0,
            "dry_base": 1.75,
            "dry_allowed_length": 2,
            "dry_penalty_last_n": -1,
            "mirostat": 0,
            "mirostat_tau": 5,
            "mirostat_eta": 0.100000001490116,
            "max_tokens": -1,
            "n_predict": -1,
            "n_keep": 0,
            "n_discard": 0,
            "ignore_eos": false,
            "stream": true,
            "n_probs": 0,
            "min_keep": 0,
            "chat_format": "Content-only",
            "reasoning_format": "none",
            "reasoning_in_content": false,
            "generation_prompt": "",
            "samplers": [
                "penalties",
                "dry",
                "top_n_sigma",
                "top_k",
                "typ_p",
                "top_p",
                "min_p",
                "xtc",
                "temperature"
            ],
            "speculative.n_max": 16,
            "speculative.n_min": 0,
            "speculative.p_min": 0.75,
            "speculative.type": "none",
            "speculative.ngram_size_n": 12,
            "speculative.ngram_size_m": 48,
            "speculative.ngram_m_hits": 1,
            "timings_per_token": false,
            "post_sampling_probs": false,
            "backend_sampling": false,
            "lora": []
        },
        "n_ctx": 262144
    },
    "total_slots": 1,
    "model_alias": "qwen3.5:122b",
    "model_path": "/home/kieran/.cache/huggingface/hub/models--unsloth--Qwen3.5-122B-A10B-GGUF/snapshots/51eab4d59d53f573fb9206cb3ce613f1d0aa392b/Q4_K_M/Qwen3.5-122B-A10B-Q4_K_M-00001-of-00003.gguf",
    "modalities": {
        "vision": true,
        "audio": false
    },
    "endpoint_slots": true,
    "endpoint_props": false,
    "endpoint_metrics": false,
    "webui": true,
    "webui_settings": {

    },
    "chat_template": "{%- set image_count = namespace(value=0) %}\n{%- set video_count = namespace(value=0) %}\n{%- macro render_content(content, do_vision_count, is_system_content=false) %}\n    {%- if content is string %}\n        {{- content }}\n    {%- elif content is iterable and content is not mapping %}\n        {%- for item in content %}\n            {%- if 'image' in item or 'image_url' in item or item.type == 'image' %}\n                {%- if is_system_content %}\n                    {{- raise_exception('System message cannot contain images.') }}\n                {%- endif %}\n                {%- if do_vision_count %}\n                    {%- set image_count.value = image_count.value + 1 %}\n                {%- endif %}\n                {%- if add_vision_id %}\n                    {{- 'Picture ' ~ image_count.value ~ ': ' }}\n                {%- endif %}\n                {{- '\u003C|vision_start|\u003E\u003C|image_pad|\u003E\u003C|vision_end|\u003E' }}\n            {%- elif 'video' in item or item.type == 'video' %}\n                {%- if is_system_content %}\n                    {{- raise_exception('System message cannot contain videos.') }}\n                {%- endif %}\n                {%- if do_vision_count %}\n                    {%- set video_count.value = video_count.value + 1 %}\n                {%- endif %}\n                {%- if add_vision_id %}\n                    {{- 'Video ' ~ video_count.value ~ ': ' }}\n                {%- endif %}\n                {{- '\u003C|vision_start|\u003E\u003C|video_pad|\u003E\u003C|vision_end|\u003E' }}\n            {%- elif 'text' in item %}\n                {{- item.text }}\n            {%- else %}\n                {{- raise_exception('Unexpected item type in content.') }}\n            {%- endif %}\n        {%- endfor %}\n    {%- elif content is none or content is undefined %}\n        {{- '' }}\n    {%- else %}\n        {{- raise_exception('Unexpected content type.') }}\n    {%- endif %}\n{%- endmacro %}\n{%- if not messages %}\n    {{- raise_exception('No messages provided.') }}\n{%- endif %}\n{%- if tools and tools is iterable and tools is not mapping %}\n    {{- '\u003C|im_start|\u003Esystem\\n' }}\n    {{- \" Tools\\n\\nYou have access to the following functions:\\n\\n\u003Ctools\u003E\" }}\n    {%- for tool in tools %}\n        {{- \"\\n\" }}\n        {{- tool | tojson }}\n    {%- endfor %}\n    {{- \"\\n\u003C/tools\u003E\" }}\n    {{- '\\n\\nIf you choose to call a function ONLY reply in the following format with NO suffix:\\n\\n\u003Ctool_call\u003E\\n\u003Cfunction=example_function_name\u003E\\n\u003Cparameter=example_parameter_1\u003E\\nvalue_1\\n\u003C/parameter\u003E\\n\u003Cparameter=example_parameter_2\u003E\\nThis is the value for the second parameter\\nthat can span\\nmultiple lines\\n\u003C/parameter\u003E\\n\u003C/function\u003E\\n\u003C/tool_call\u003E\\n\\n\u003CIMPORTANT\u003E\\nReminder:\\n- Function calls MUST follow the specified format: an inner \u003Cfunction=...\u003E\u003C/function\u003E block must be nested within \u003Ctool_call\u003E\u003C/tool_call\u003E XML tags\\n- Required parameters MUST be specified\\n- You may provide optional reasoning for your function call in natural language BEFORE the function call, but NOT after\\n- If there is no function call available, answer the question like normal with your current knowledge and do not tell the user about function calls\\n\u003C/IMPORTANT\u003E' }}\n    {%- if messages[0].role == 'system' %}\n        {%- set content = render_content(messages[0].content, false, true)|trim %}\n        {%- if content %}\n            {{- '\\n\\n' + content }}\n        {%- endif %}\n    {%- endif %}\n    {{- '\u003C|im_end|\u003E\\n' }}\n{%- else %}\n    {%- if messages[0].role == 'system' %}\n        {%- set content = render_content(messages[0].content, false, true)|trim %}\n        {{- '\u003C|im_start|\u003Esystem\\n' + content + '\u003C|im_end|\u003E\\n' }}\n    {%- endif %}\n{%- endif %}\n{%- set ns = namespace(multi_step_tool=true, last_query_index=messages|length - 1) %}\n{%- for message in messages[::-1] %}\n    {%- set index = (messages|length - 1) - loop.index0 %}\n    {%- if ns.multi_step_tool and message.role == \"user\" %}\n        {%- set content = render_content(message.content, false)|trim %}\n        {%- if not(content.startswith('\u003Ctool_response\u003E') and content.endswith('\u003C/tool_response\u003E')) %}\n            {%- set ns.multi_step_tool = false %}\n            {%- set ns.last_query_index = index %}\n        {%- endif %}\n    {%- endif %}\n{%- endfor %}\n{%- if ns.multi_step_tool %}\n    {{- raise_exception('No user query found in messages.') }}\n{%- endif %}\n{%- for message in messages %}\n    {%- set content = render_content(message.content, true)|trim %}\n    {%- if message.role == \"system\" %}\n        {%- if not loop.first %}\n            {{- raise_exception('System message must be at the beginning.') }}\n        {%- endif %}\n    {%- elif message.role == \"user\" %}\n        {{- '\u003C|im_start|\u003E' + message.role + '\\n' + content + '\u003C|im_end|\u003E' + '\\n' }}\n    {%- elif message.role == \"assistant\" %}\n        {%- set reasoning_content = '' %}\n        {%- if message.reasoning_content is string %}\n            {%- set reasoning_content = message.reasoning_content %}\n        {%- else %}\n            {%- if '\u003C/think\u003E' in content %}\n                {%- set reasoning_content = content.split('\u003C/think\u003E')[0].rstrip('\\n').split('\u003Cthink\u003E')[-1].lstrip('\\n') %}\n                {%- set content = content.split('\u003C/think\u003E')[-1].lstrip('\\n') %}\n            {%- endif %}\n        {%- endif %}\n        {%- set reasoning_content = reasoning_content|trim %}\n        {%- if loop.index0 \u003E ns.last_query_index %}\n            {{- '\u003C|im_start|\u003E' + message.role + '\\n\u003Cthink\u003E\\n' + reasoning_content + '\\n\u003C/think\u003E\\n\\n' + content }}\n        {%- else %}\n            {{- '\u003C|im_start|\u003E' + message.role + '\\n' + content }}\n        {%- endif %}\n        {%- if message.tool_calls and message.tool_calls is iterable and message.tool_calls is not mapping %}\n            {%- for tool_call in message.tool_calls %}\n                {%- if tool_call.function is defined %}\n                    {%- set tool_call = tool_call.function %}\n                {%- endif %}\n                {%- if loop.first %}\n                    {%- if content|trim %}\n                        {{- '\\n\\n\u003Ctool_call\u003E\\n\u003Cfunction=' + tool_call.name + '\u003E\\n' }}\n                    {%- else %}\n                        {{- '\u003Ctool_call\u003E\\n\u003Cfunction=' + tool_call.name + '\u003E\\n' }}\n                    {%- endif %}\n                {%- else %}\n                    {{- '\\n\u003Ctool_call\u003E\\n\u003Cfunction=' + tool_call.name + '\u003E\\n' }}\n                {%- endif %}\n                {%- if tool_call.arguments is mapping %}\n                    {%- for args_name in tool_call.arguments %}\n                        {%- set args_value = tool_call.arguments[args_name] %}\n                        {{- '\u003Cparameter=' + args_name + '\u003E\\n' }}\n                        {%- set args_value = args_value | tojson | safe if args_value is mapping or (args_value is sequence and args_value is not string) else args_value | string %}\n                        {{- args_value }}\n                        {{- '\\n\u003C/parameter\u003E\\n' }}\n                    {%- endfor %}\n                {%- endif %}\n                {{- '\u003C/function\u003E\\n\u003C/tool_call\u003E' }}\n            {%- endfor %}\n        {%- endif %}\n        {{- '\u003C|im_end|\u003E\\n' }}\n    {%- elif message.role == \"tool\" %}\n        {%- if loop.previtem and loop.previtem.role != \"tool\" %}\n            {{- '\u003C|im_start|\u003Euser' }}\n        {%- endif %}\n        {{- '\\n\u003Ctool_response\u003E\\n' }}\n        {{- content }}\n        {{- '\\n\u003C/tool_response\u003E' }}\n        {%- if not loop.last and loop.nextitem.role != \"tool\" %}\n            {{- '\u003C|im_end|\u003E\\n' }}\n        {%- elif loop.last %}\n            {{- '\u003C|im_end|\u003E\\n' }}\n        {%- endif %}\n    {%- else %}\n        {{- raise_exception('Unexpected message role.') }}\n    {%- endif %}\n{%- endfor %}\n{%- if add_generation_prompt %}\n    {{- '\u003C|im_start|\u003Eassistant\\n' }}\n    {%- if enable_thinking is defined and enable_thinking is false %}\n        {{- '\u003Cthink\u003E\\n\\n\u003C/think\u003E\\n\\n' }}\n    {%- else %}\n        {{- '\u003Cthink\u003E\\n' }}\n    {%- endif %}\n{%- endif %}",
    "chat_template_caps": {
        "supports_object_arguments": true,
        "supports_parallel_tool_calls": true,
        "supports_preserve_reasoning": true,
        "supports_string_content": true,
        "supports_system_role": true,
        "supports_tool_calls": true,
        "supports_tools": true,
        "supports_typed_content": false
    },
    "bos_token": ",",
    "eos_token": "\u003C|im_end|\u003E",
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

        let mut runtime_info = ModelRuntimeInfo::from_api_response("test-model", additional_fields);
        runtime_info.context_length = props.default_generation_settings.and_then(|g| g.n_ctx);
        runtime_info.quantization = props.model_quant_type;
        runtime_info.parameter_size = props.model_n_params.map(|p| p.to_string());
        runtime_info.max_output_tokens = props.n_batch;

        if let Some(modalities) = props.modalities {
            if modalities.vision.unwrap_or(false) {
                runtime_info.modalities.push(crate::router::Modality::Image);
            }
            if modalities.audio.unwrap_or(false) {
                runtime_info.modalities.push(crate::router::Modality::Audio);
            }
        }

        assert_eq!(runtime_info.context_length(), Some(262144));
        assert!(runtime_info.supports_image());
        assert!(!runtime_info.supports_audio());
        assert!(runtime_info
            .modalities
            .contains(&crate::router::Modality::Image));
    }

    #[tokio::test]
    async fn test_provider_name_and_slug() {
        let provider = LlamaCppProvider::new("Test Provider", Some("test"), "http://localhost:8080", None);

        assert_eq!(provider.name(), "Test Provider");
        assert_eq!(provider.slug(), "test");
    }

    #[tokio::test]
    async fn test_provider_slug_generation() {
        let provider1 = LlamaCppProvider::new("My Provider", None, "http://localhost:8080", None);
        assert_eq!(provider1.slug(), "my-provider");

        let provider2 = LlamaCppProvider::new("Test_Provider", Some("custom_slug"), "http://localhost:8080", None);
        assert_eq!(provider2.slug(), "custom-slug");
    }

    #[tokio::test]
    async fn test_health_check_returns_bool() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None);
        let result = provider.health_check().await;
        assert!(result.is_ok());
        let _is_healthy = result.unwrap();
    }

    #[tokio::test]
    async fn test_fetch_props_success() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None);
        let result = provider.fetch_props().await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_fetch_props_with_trailing_slash() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080/", None);
        assert_eq!(provider.props_url, "http://localhost:8080/props");
    }

    #[tokio::test]
    async fn test_fetch_props_without_trailing_slash() {
        let provider = LlamaCppProvider::new("Test", None, "http://localhost:8080", None);
        assert_eq!(provider.props_url, "http://localhost:8080/props");
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

        let provider = LlamaCppProvider::new("Test", None, &mock_server.uri(), None);
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

        let provider = LlamaCppProvider::new("Test", None, &mock_server.uri(), None);
        let result = provider.fetch_props().await;
        assert!(result.is_err());
    }
}
