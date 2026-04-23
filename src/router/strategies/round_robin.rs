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

    async fn select(&self, entries: &[ProviderEntry], _key: &str) -> Option<usize> {
        if entries.is_empty() {
            tracing::warn!("RoundRobin: No entries available for routing");
            return None;
        }

        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % entries.len();
        tracing::debug!(
            strategy = "round_robin",
            selected_provider = entries[idx].provider.name(),
            provider_index = idx,
            total_entries = entries.len(),
            "Entry selected"
        );
        Some(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{
        CreateChatCompletionRequest, CreateChatCompletionResponse,
        StreamingChunk, ProviderError, Model,
    };
    use futures::{stream, StreamExt, stream::BoxStream};

    struct TestProvider {
        name: String,
        slug: String,
    }

    impl TestProvider {
        fn new(name: &str, slug: &str) -> Self {
            Self {
                name: name.to_string(),
                slug: slug.to_string(),
            }
        }
    }

    #[async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn slug(&self) -> &str {
            &self.slug
        }

        async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
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
        ) -> Result<BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
            Ok(stream::empty().boxed())
        }

        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
    }

    fn make_entries(names: &[&str]) -> Vec<ProviderEntry> {
        names
            .iter()
            .map(|n| ProviderEntry {
                provider: Arc::new(TestProvider::new(n, n)) as Arc<dyn Provider>,
                model_override: None,
                weight: 100,
            })
            .collect()
    }

    #[tokio::test]
    async fn test_round_robin_selects_all() {
        let strategy = RoundRobinStrategy::new();
        let entries = make_entries(&["p1", "p2", "p3"]);

        let mut selections = Vec::new();
        for _ in 0..6 {
            let idx = strategy.select(&entries, "test").await.unwrap();
            selections.push(entries[idx].provider.name().to_string());
        }

        assert_eq!(
            selections,
            vec!["p1", "p2", "p3", "p1", "p2", "p3"]
        );
    }

    #[tokio::test]
    async fn test_round_robin_empty() {
        let strategy = RoundRobinStrategy::new();
        let entries: Vec<ProviderEntry> = vec![];

        assert!(strategy.select(&entries, "test").await.is_none());
    }

    #[tokio::test]
    async fn test_round_robin_single() {
        let strategy = RoundRobinStrategy::new();
        let entries = make_entries(&["p1"]);

        let mut selections = Vec::new();
        for _ in 0..3 {
            let idx = strategy.select(&entries, "test").await.unwrap();
            selections.push(entries[idx].provider.name().to_string());
        }

        assert_eq!(selections, vec!["p1", "p1", "p1"]);
    }

    #[tokio::test]
    async fn test_model_override_preserved() {
        let provider: Arc<dyn Provider> = Arc::new(TestProvider::new("p1", "p1"));
        let entries = vec![ProviderEntry {
            provider: provider.clone(),
            model_override: Some("gpt-4-0613".to_string()),
            weight: 100,
        }];

        let strategy = RoundRobinStrategy::new();
        let idx = strategy.select(&entries, "gpt-4").await.unwrap();
        assert_eq!(entries[idx].model_override, Some("gpt-4-0613".to_string()));
    }
}
