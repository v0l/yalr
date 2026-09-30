use crate::metrics::MetricsUser;
use crate::providers::decision::{DecisionRequest, DecisionResponse};
use crate::router::engine::{Router, RouterError};

impl Router {
    pub async fn decide(
        &self,
        request: &DecisionRequest,
        user: Option<MetricsUser>,
    ) -> Result<DecisionResponse, RouterError> {
        let emitter = self.metrics_store.emitter().clone();
        let requested_model = request.model.as_str();
        self.capability_failover(requested_model, "decisions", user.clone(), |provider, model| {
            let mut request = request.clone();
            request.model = model;
            let emitter = emitter.clone();
            let user = user.clone();
            async move {
                let response = provider.decide(&request).await?;
                if let Some(usage) = response.usage() {
                    emitter.emit_input_tokens(provider.name(), requested_model, usage.input_tokens, user.clone());
                    emitter.emit_output_tokens(provider.name(), requested_model, usage.output_tokens, user);
                }
                Ok(response)
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::metrics::MetricsStore;
    use crate::providers::Provider;
    use crate::ProviderError;
    use async_trait::async_trait;
    use bytes::Bytes;
    use std::sync::{Arc, Mutex};

    struct FakeDecider {
        name: String,
        supported: bool,
        client_error: bool,
        seen_models: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Provider for FakeDecider {
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
            _request: &crate::providers::ChatRequest,
        ) -> Result<crate::ChatCompletionResponse, ProviderError> {
            Err(ProviderError::Timeout)
        }
        fn chat_completions_stream(
            &self,
            _request: &crate::providers::ChatRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<crate::providers::StreamingChunk, ProviderError>>,
            ProviderError,
        > {
            Err(ProviderError::Timeout)
        }
        async fn health_check(&self) -> Result<bool, ProviderError> {
            Ok(true)
        }
        async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse, ProviderError> {
            self.seen_models.lock().unwrap().push(request.model.clone());
            if self.client_error {
                return Err(ProviderError::ServerError {
                    message: "state exceeds 2048 tokens".into(),
                    status_code: Some(422),
                });
            }
            if !self.supported {
                return Err(ProviderError::Unsupported("chat only".into()));
            }
            DecisionResponse::parse(Bytes::from_static(
                br#"{"model":"jev","answers":{"r":{"type":"noul","noul":0.9}},"usage":{"input_tokens":12,"output_tokens":3}}"#,
            ))
            .map_err(ProviderError::Unsupported)
        }
    }

    fn fake(name: &str, supported: bool) -> (Arc<dyn Provider>, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(FakeDecider {
                name: name.into(),
                supported,
                client_error: false,
                seen_models: seen.clone(),
            }),
            seen,
        )
    }

    fn request(model: &str) -> DecisionRequest {
        serde_json::from_str(&format!(
            r#"{{"model":"{model}","state":"refund me","questions":{{"r":{{"type":"noul","instructions":"Refund?"}}}}}}"#
        ))
        .unwrap()
    }

    async fn router() -> Router {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        Router::new(MetricsStore::new(100), db)
    }

    #[tokio::test]
    async fn skips_chat_only_backend_in_a_mixed_pool() {
        let (chat, chat_seen) = fake("chat", false);
        let (jev, jev_seen) = fake("jev", true);
        let router = router().await;
        router.register_route("jev", vec![chat, jev]).await;

        let response = router.decide(&request("jev"), None).await.unwrap();
        assert_eq!(response.usage().unwrap().input_tokens, 12);
        assert_eq!(chat_seen.lock().unwrap().len(), 1);
        assert_eq!(jev_seen.lock().unwrap().len(), 1);
        assert_eq!(router.metrics_store.get_recent_failures("chat").await, 0);
    }

    #[tokio::test]
    async fn prefixed_model_reaches_the_provider_without_its_slug() {
        let (openrouter, seen) = fake("openrouter", true);
        let router = router().await;
        router.add_provider(openrouter).await;

        router
            .decide(&request("openrouter/typesafe/jev-1.13"), None)
            .await
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["typesafe/jev-1.13".to_string()]);
    }

    #[tokio::test]
    async fn no_decision_backend_reports_unsupported() {
        let (chat, _) = fake("chat", false);
        let router = router().await;
        router.register_route("jev", vec![chat]).await;

        let err = router.decide(&request("jev"), None).await.unwrap_err();
        assert!(
            matches!(err, RouterError::ProviderError(ProviderError::Unsupported(_))),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn rejected_request_leaves_provider_health_alone() {
        let provider: Arc<dyn Provider> = Arc::new(FakeDecider {
            name: "jev".into(),
            supported: true,
            client_error: true,
            seen_models: Default::default(),
        });
        let router = router().await;
        router.register_route("jev", vec![provider]).await;

        for _ in 0..6 {
            assert!(router.decide(&request("jev"), None).await.is_err());
        }
        assert_eq!(router.metrics_store.get_recent_failures("jev").await, 0);
        assert!(router.metrics_store.is_provider_available("jev").await);
    }
}
