use super::*;
use crate::db::{NewProvider, NewRoutingConfig, NewRoutingConfigProvider, ProviderType};

fn config(name: &str) -> NewRoutingConfig {
    NewRoutingConfig {
        name: name.into(),
        strategy: "round_robin".into(),
        health_check_enabled: false,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }
}

async fn assign(db: &Database, config_id: i64, provider_id: i64) {
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: config_id,
        provider_id,
        model: None,
        weight: 1,
        is_active: true,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn an_owned_provider_routes_only_inside_its_owners_configs() {
    let db = Arc::new(Database::test().await);
    let alice = db.add_test_user("alice", false).await;
    let bob = db.add_test_user("bob", false).await;
    let provider = db
        .create_owned_provider(
            alice.id,
            NewProvider {
                name: "alice/gpu",
                slug: "u-alice-gpu",
                base_url: "http://127.0.0.1:9/v1",
                api_key: None,
                provider_type: Some(ProviderType::OpenAi),
            },
        )
        .await
        .unwrap();
    let mine = db.create_owned_routing_config(Some(alice.id), config("alice/fast")).await.unwrap();
    let theirs = db.create_owned_routing_config(Some(bob.id), config("bob/fast")).await.unwrap();
    let platform = db.create_owned_routing_config(None, config("fast")).await.unwrap();
    for id in [mine.id, theirs.id, platform.id] {
        assign(&db, id, provider.id).await;
    }

    let router = Router::new(MetricsStore::new(100), db);
    router.reload_config().await.unwrap();

    let names = |backends: Vec<(Arc<dyn Provider>, String)>| -> Vec<String> {
        backends.into_iter().map(|(p, _)| p.slug().to_string()).collect()
    };
    assert_eq!(names(router.candidate_backends("alice/fast").await), vec!["u-alice-gpu"]);
    assert!(router.candidate_backends("bob/fast").await.is_empty());
    assert!(router.candidate_backends("fast").await.is_empty());
}

struct Echo(&'static str);

#[async_trait::async_trait]
impl Provider for Echo {
    fn name(&self) -> &str {
        self.0
    }
    fn slug(&self) -> &str {
        self.0
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![crate::Model { id: "acme/m-1".into(), object: "model".into(), created: 0, owned_by: "x".into() }])
    }
    async fn chat_completions(&self, _request: &ChatRequest) -> Result<ChatCompletionResponse, ProviderError> {
        Err(ProviderError::Unsupported("not called".into()))
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        Err(ProviderError::Unsupported("not called".into()))
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

#[tokio::test]
async fn approval_status_and_owner_configs_both_gate_a_user_provider() {
    let db = Arc::new(Database::test().await);
    let alice = db.add_test_user("alice", false).await;
    let bob = db.add_test_user("bob", false).await;
    let provider = db
        .create_owned_provider(
            alice.id,
            NewProvider {
                name: "alice/gpu",
                slug: "u-alice-gpu",
                base_url: "http://127.0.0.1:9/v1",
                api_key: None,
                provider_type: Some(ProviderType::OpenAi),
            },
        )
        .await
        .unwrap();
    assert_eq!(provider.status, crate::db::PENDING);
    let mine = db.create_owned_routing_config(Some(alice.id), config("alice/fast")).await.unwrap();
    let theirs = db.create_owned_routing_config(Some(bob.id), config("bob/fast")).await.unwrap();
    assign(&db, mine.id, provider.id).await;
    assign(&db, theirs.id, provider.id).await;

    let router = Router::new(MetricsStore::new(100), db.clone());
    router.reload_config().await.unwrap();
    router.add_provider(Arc::new(Echo("u-alice-gpu"))).await;
    let routed = |model: &'static str, viewer: Option<i64>| {
        let router = &router;
        async move {
            router
                .collect_routed(model, None, viewer)
                .await
                .unwrap()
                .into_iter()
                .map(|c| c.provider.slug().to_string())
                .collect::<Vec<_>>()
        }
    };

    assert_eq!(routed("alice/fast", Some(alice.id)).await, vec!["u-alice-gpu"]);
    assert!(routed("alice/fast", Some(bob.id)).await.is_empty());
    assert!(routed("alice/fast", None).await.is_empty());
    assert!(routed("bob/fast", Some(bob.id)).await.is_empty());
    assert!(routed("bob/fast", Some(alice.id)).await.is_empty());
    assert_eq!(routed("u-alice-gpu/m-1", Some(alice.id)).await, vec!["u-alice-gpu"]);
    assert!(routed("u-alice-gpu/m-1", Some(bob.id)).await.is_empty());

    db.set_provider_status(provider.id, crate::db::APPROVED, None, false).await.unwrap();
    router.reload_config().await.unwrap();
    router.add_provider(Arc::new(Echo("u-alice-gpu"))).await;
    assert_eq!(routed("alice/fast", Some(bob.id)).await, vec!["u-alice-gpu"]);
    assert!(routed("bob/fast", Some(bob.id)).await.is_empty());

    db.set_provider_status(provider.id, crate::db::SUSPENDED, Some("failed"), false).await.unwrap();
    router.reload_config().await.unwrap();
    router.add_provider(Arc::new(Echo("u-alice-gpu"))).await;
    assert!(routed("alice/fast", Some(alice.id)).await.is_empty());
    assert!(routed("u-alice-gpu/m-1", Some(alice.id)).await.is_empty());
}
