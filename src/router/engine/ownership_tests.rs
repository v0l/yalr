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
