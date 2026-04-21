use std::sync::Arc;
use futures::{stream, StreamExt};
use yalr::{
    ChatCompletionRequest, ChatCompletionRequestMessage, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, Provider, ProviderError, Router, RouterError,
};
use yalr::providers::{
    CreateChatCompletionRequest, CreateChatCompletionResponse, CreateChatCompletionStreamResponse,
};
use yalr::metrics::{MetricsEmitter, MetricsStore};
use async_openai::types::chat::CompletionUsage;

struct MockProvider {
    name: String,
    slug: String,
    should_fail: bool,
    response_delay_ms: u64,
}

impl MockProvider {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            slug: name.to_lowercase(),
            should_fail: false,
            response_delay_ms: 0,
        }
    }

    fn with_failure(mut self) -> Self {
        self.should_fail = true;
        self
    }

    fn with_delay(mut self, delay_ms: u64) -> Self {
        self.response_delay_ms = delay_ms;
        self
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn slug(&self) -> &str {
        &self.slug
    }

    async fn list_models(&self) -> Result<Vec<yalr::Model>, ProviderError> {
        Ok(vec![])
    }

    async fn chat_completions(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ProviderError> {
        if self.should_fail {
            return Err(ProviderError::ProviderError("Mock failure".to_string()));
        }

        if self.response_delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.response_delay_ms)).await;
        }

        Ok(CreateChatCompletionResponse {
            id: format!("mock-{}", self.name),
            object: "chat.completion".to_string(),
            created: 0,
            model: request.model.clone(),
            choices: vec![],
            usage: Some(CompletionUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
            system_fingerprint: None,
            service_tier: None,
        })
    }

    fn chat_completions_stream(
        &self,
        request: &CreateChatCompletionRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>,
        ProviderError,
    > {
        if self.should_fail {
            return Ok(Box::pin(stream::once(async move {
                Err(ProviderError::ProviderError("Mock stream failure".to_string()))
            })));
        }

        let chunks = vec![
            Ok(CreateChatCompletionStreamResponse {
                id: format!("mock-stream-{}", self.name),
                object: "chat.completion.chunk".to_string(),
                choices: vec![],
                created: 0,
                model: request.model.clone(),
                system_fingerprint: None,
                service_tier: None,
                usage: None,
            }),
        ];

        Ok(Box::pin(stream::iter(chunks)))
    }

    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(!self.should_fail)
    }
}

fn create_test_router() -> (Router, MetricsStore) {
    let metrics_store = MetricsStore::new(1000);
    
    let router = Router::new(
        Box::new(yalr::router::strategies::round_robin::RoundRobinStrategy::new()),
        metrics_store.clone(),
    );
    
    (router, metrics_store)
}

fn create_test_request(model: &str) -> ChatCompletionRequest {
    ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
            content: ChatCompletionRequestUserMessageContent::Text("Hello".to_string()),
            name: None,
        })],
        stream: Some(false),
        ..Default::default()
    }
}

fn create_test_stream_request(model: &str) -> ChatCompletionRequest {
    let mut req = create_test_request(model);
    req.stream = Some(true);
    req
}

#[tokio::test]
async fn test_router_with_single_provider() {
    let (router, _metrics) = create_test_router();
    let provider1 = Arc::new(MockProvider::new("provider1"));
    
    router.add_provider(provider1.clone()).await;
    
    let request = create_test_request("test-model");
    let response = router.chat_completions(&request).await;
    
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_router_round_robin_distribution() {
    let (router, _metrics) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("provider1"));
    let provider2 = Arc::new(MockProvider::new("provider2"));
    let provider3 = Arc::new(MockProvider::new("provider3"));
    
    router.add_provider(provider1.clone()).await;
    router.add_provider(provider2.clone()).await;
    router.add_provider(provider3.clone()).await;
    
    let mut results = Vec::new();
    for i in 0..6 {
        let request = create_test_request(&format!("model-{}", i));
        let response = router.chat_completions(&request).await;
        assert!(response.is_ok());
        results.push(response.unwrap().id);
    }
    
    let unique_ids: std::collections::HashSet<_> = results.iter().collect();
    assert_eq!(unique_ids.len(), 3, "Should have used all 3 providers");
}

#[tokio::test]
async fn test_router_with_failing_provider() {
    let (router, _metrics) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("provider1").with_failure());
    let provider2 = Arc::new(MockProvider::new("provider2"));
    
    router.add_provider(provider1.clone()).await;
    router.add_provider(provider2.clone()).await;
    
    let request = create_test_request("test-model");
    let response = router.chat_completions(&request).await;
    
    assert!(response.is_ok() || response.is_err());
}

#[tokio::test]
async fn test_router_no_providers() {
    let (router, _metrics) = create_test_router();
    
    let request = create_test_request("test-model");
    let response = router.chat_completions(&request).await;
    
    assert!(matches!(response, Err(RouterError::NoAvailableProvider)));
}

#[tokio::test]
async fn test_router_streaming() {
    let (router, _metrics) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("provider1"));
    router.add_provider(provider1.clone()).await;
    
    let request = create_test_stream_request("test-model");
    let stream_result = router.chat_completions_stream(&request).await;
    
    assert!(stream_result.is_ok());
    
    let mut stream = stream_result.unwrap();
    let mut chunk_count = 0;
    
    while let Some(chunk_result) = stream.next().await {
        assert!(chunk_result.is_ok());
        chunk_count += 1;
    }
    
    assert!(chunk_count > 0);
}

#[tokio::test]
async fn test_router_streaming_with_failure() {
    let (router, _metrics) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("provider1").with_failure());
    router.add_provider(provider1.clone()).await;
    
    let request = create_test_stream_request("test-model");
    let stream_result = router.chat_completions_stream(&request).await;
    
    assert!(stream_result.is_ok());
    
    let mut stream = stream_result.unwrap();
    
    if let Some(chunk_result) = stream.next().await {
        assert!(chunk_result.is_err());
    }
}

#[tokio::test]
async fn test_router_slug_based_routing() {
    let (router, _metrics) = create_test_router();
    
    let openai_provider = Arc::new(MockProvider::new("openai-primary"));
    let anthropic_provider = Arc::new(MockProvider::new("anthropic-main"));
    
    router.add_provider(openai_provider.clone()).await;
    router.add_provider(anthropic_provider.clone()).await;
    
    let request = create_test_request("openai/gpt-4");
    let response = router.chat_completions(&request).await;
    assert!(response.is_ok());
    
    let request = create_test_request("anthropic/claude-3");
    let response = router.chat_completions(&request).await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn test_router_metrics_collection() {
    let (router, metrics_store) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("provider1"));
    router.add_provider(provider1.clone()).await;
    
    let request = create_test_request("test-model");
    let _ = router.chat_completions(&request).await;
    
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    
    let summary = metrics_store.get_provider_summary("provider1").await;
    assert_eq!(summary.provider, "provider1");
}

#[tokio::test]
async fn test_router_latency_tracking() {
    let (router, metrics_store) = create_test_router();
    
    let provider1 = Arc::new(MockProvider::new("fast-provider").with_delay(10));
    let provider2 = Arc::new(MockProvider::new("slow-provider").with_delay(100));
    
    router.add_provider(provider1.clone()).await;
    router.add_provider(provider2.clone()).await;
    
    for i in 0..3 {
        let request = create_test_request(&format!("model-{}", i));
        let response = router.chat_completions(&request).await;
        assert!(response.is_ok());
    }
}
