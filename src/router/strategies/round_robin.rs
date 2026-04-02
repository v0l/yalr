use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct RoundRobinStrategy {
    counter: AtomicUsize,
}

impl RoundRobinStrategy {
    pub fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

impl Default for RoundRobinStrategy {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RoutingStrategy for RoundRobinStrategy {
    fn name(&self) -> &str {
        "round_robin"
    }

    async fn select_provider(
        &self,
        providers: &[Arc<dyn Provider>],
        _model: &str,
    ) -> Option<Arc<dyn Provider>> {
        if providers.is_empty() {
            tracing::warn!("RoundRobin: No providers available for routing");
            return None;
        }

        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % providers.len();
        let selected = &providers[idx];
        tracing::debug!(
            strategy = "round_robin",
            selected_provider = selected.name(),
            provider_index = idx,
            total_providers = providers.len(),
            "Provider selected"
        );
        Some(selected.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        CreateChatCompletionRequest, CreateChatCompletionResponse,
        CreateChatCompletionStreamResponse, ProviderError,
    };
    use futures::{stream, StreamExt, stream::BoxStream};

    struct TestProvider {
        name: String,
    }

    impl TestProvider {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
            }
        }
    }

    #[async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn slug(&self) -> &str {
            &self.name
        }

        async fn list_models(&self) -> Result<Vec<crate::providers::Model>, ProviderError> {
            Ok(vec![])
        }

        async fn chat_completions(
            &self,
            _request: &CreateChatCompletionRequest,
        ) -> Result<CreateChatCompletionResponse, ProviderError> {
            unimplemented!()
        }

        fn chat_completions_stream(
            &self,
            _request: &CreateChatCompletionRequest,
        ) -> Result<BoxStream<'static, Result<CreateChatCompletionStreamResponse, ProviderError>>, ProviderError> {
            Ok(stream::empty().boxed())
        }

        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn test_round_robin_selects_all_providers() {
        let strategy = Arc::new(RoundRobinStrategy::new());
        
        let provider1: Arc<dyn Provider> = Arc::new(TestProvider::new("provider1"));
        let provider2: Arc<dyn Provider> = Arc::new(TestProvider::new("provider2"));
        let provider3: Arc<dyn Provider> = Arc::new(TestProvider::new("provider3"));
        
        let providers: Vec<Arc<dyn Provider>> = vec![provider1, provider2, provider3];
        
        let mut selections = Vec::new();
        for _ in 0..6 {
            let p = strategy.select_provider(&providers, "test-model").await;
            selections.push(p.unwrap().name().to_string());
        }
        
        assert_eq!(selections, vec!["provider1", "provider2", "provider3", "provider1", "provider2", "provider3"]);
    }

    #[tokio::test]
    async fn test_round_robin_empty_providers() {
        let strategy = Arc::new(RoundRobinStrategy::new());
        let providers: Vec<Arc<dyn Provider>> = vec![];
        
        let p = strategy.select_provider(&providers, "test-model").await;
        assert!(p.is_none());
    }

    #[tokio::test]
    async fn test_round_robin_single_provider() {
        let strategy = Arc::new(RoundRobinStrategy::new());
        
        let provider1: Arc<dyn Provider> = Arc::new(TestProvider::new("provider1"));
        let providers: Vec<Arc<dyn Provider>> = vec![provider1];
        
        let mut selections = Vec::new();
        for _ in 0..3 {
            let p = strategy.select_provider(&providers, "test-model").await;
            selections.push(p.unwrap().name().to_string());
        }
        
        assert_eq!(selections, vec!["provider1", "provider1", "provider1"]);
    }
}
