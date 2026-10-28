use crate::db::*;
use super::setup_test_db;

// Provider Tests
#[tokio::test]
async fn test_create_provider() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let result = db.create_provider(provider).await.unwrap();
    assert_eq!(result.name, "test-provider");
    assert_eq!(result.slug, "test");
    assert_eq!(result.base_url, "http://localhost:8080");
    assert_eq!(result.api_key, Some("test-key".to_string()));
}

#[tokio::test]
async fn test_create_provider_without_api_key() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    };
    let result = db.create_provider(provider).await.unwrap();
    assert_eq!(result.api_key, None);
}

#[tokio::test]
async fn test_get_provider_by_id() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let created = db.create_provider(provider).await.unwrap();
    let found = db.get_provider_by_id(created.id).await.unwrap().unwrap();
    assert_eq!(found.id, created.id);
    assert_eq!(found.name, created.name);
}

#[tokio::test]
async fn test_get_provider_by_id_not_found() {
    let db = setup_test_db().await;
    let result = db.get_provider_by_id(999).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_provider_by_slug() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let created = db.create_provider(provider).await.unwrap();
    let found = db.get_provider_by_slug("test").await.unwrap().unwrap();
    assert_eq!(found.id, created.id);
}

#[tokio::test]
async fn test_get_provider_by_slug_not_found() {
    let db = setup_test_db().await;
    let result = db.get_provider_by_slug("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_list_providers() {
    let db = setup_test_db().await;
    db.create_provider(NewProvider {
        name: "provider1",
        slug: "p1",
        base_url: "http://localhost:1",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    db.create_provider(NewProvider {
        name: "provider2",
        slug: "p2",
        base_url: "http://localhost:2",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let providers = db.list_providers().await.unwrap();
    assert_eq!(providers.len(), 2);
}

#[tokio::test]
async fn test_update_provider() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let created = db.create_provider(provider).await.unwrap();
    let updated = db.update_provider(created.id, UpdateProvider {
        name: Some("updated-name"),
        slug: Some("updated-slug"),
        base_url: Some("http://localhost:9090"),
        api_key: Some(Some("new-key")),
        provider_type: None,
    }).await.unwrap();
    assert_eq!(updated.name, "updated-name");
    assert_eq!(updated.slug, "updated-slug");
    assert_eq!(updated.base_url, "http://localhost:9090");
    assert_eq!(updated.api_key, Some("new-key".to_string()));
}

#[tokio::test]
async fn test_set_provider_client_version() {
    let db = setup_test_db().await;
    let created = db.create_provider(NewProvider {
        name: "claude",
        slug: "claude",
        base_url: "https://api.anthropic.com",
        api_key: None,
        provider_type: Some(ProviderType::AnthropicOauth),
    }).await.unwrap();
    assert_eq!(created.client_version, None);
    let set = db.set_provider_client_version(created.id, Some("2.1.301")).await.unwrap();
    assert_eq!(set.client_version.as_deref(), Some("2.1.301"));
    let cleared = db.set_provider_client_version(created.id, None).await.unwrap();
    assert_eq!(cleared.client_version, None);
}

#[tokio::test]
async fn test_update_provider_partial() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let created = db.create_provider(provider).await.unwrap();
    let updated = db.update_provider(created.id, UpdateProvider {
        name: Some("updated-name"),
        slug: None,
        base_url: None,
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    assert_eq!(updated.name, "updated-name");
    assert_eq!(updated.slug, "test");
    assert_eq!(updated.base_url, "http://localhost:8080");
}

#[tokio::test]
async fn test_delete_provider() {
    let db = setup_test_db().await;
    let provider = NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: Some("test-key"),
        provider_type: None,
    };
    let created = db.create_provider(provider).await.unwrap();
    let deleted = db.delete_provider(created.id).await.unwrap();
    assert!(deleted);
    let found = db.get_provider_by_id(created.id).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn test_delete_provider_not_found() {
    let db = setup_test_db().await;
    let deleted = db.delete_provider(999).await.unwrap();
    assert!(!deleted);
}

// Model Tests
#[tokio::test]
async fn test_create_model() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let result = db.create_model(model).await.unwrap();
    assert_eq!(result.name, "test-model");
    assert_eq!(result.cost_per_1m_input, 0.01);
    assert_eq!(result.cost_per_1m_output, 0.02);
}

#[tokio::test]
async fn test_get_model_by_id() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let created = db.create_model(model).await.unwrap();
    let found = db.get_model_by_id(created.id).await.unwrap().unwrap();
    assert_eq!(found.id, created.id);
}

#[tokio::test]
async fn test_get_model_by_name() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let created = db.create_model(model).await.unwrap();
    let found = db.get_model_by_name("test-model").await.unwrap().unwrap();
    assert_eq!(found.id, created.id);
}

#[tokio::test]
async fn test_list_models() {
    let db = setup_test_db().await;
    db.create_model(NewModel {
        name: "model1",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    db.create_model(NewModel {
        name: "model2",
        cost_per_1m_input: 0.03,
        cost_per_1m_output: 0.04,
    }).await.unwrap();
    let models = db.list_models().await.unwrap();
    assert_eq!(models.len(), 2);
}

#[tokio::test]
async fn test_update_model() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let created = db.create_model(model).await.unwrap();
    let updated = db.update_model(created.id, UpdateModel {
        name: Some("updated-model"),
        cost_per_1m_input: Some(0.05),
        cost_per_1m_output: Some(0.10),
    }).await.unwrap();
    assert_eq!(updated.name, "updated-model");
    assert_eq!(updated.cost_per_1m_input, 0.05);
    assert_eq!(updated.cost_per_1m_output, 0.10);
}

#[tokio::test]
async fn test_update_model_partial() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let created = db.create_model(model).await.unwrap();
    let updated = db.update_model(created.id, UpdateModel {
        name: None,
        cost_per_1m_input: Some(0.05),
        cost_per_1m_output: None,
    }).await.unwrap();
    assert_eq!(updated.name, "test-model");
    assert_eq!(updated.cost_per_1m_input, 0.05);
    assert_eq!(updated.cost_per_1m_output, 0.02);
}

#[tokio::test]
async fn test_delete_model() {
    let db = setup_test_db().await;
    let model = NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    };
    let created = db.create_model(model).await.unwrap();
    let deleted = db.delete_model(created.id).await.unwrap();
    assert!(deleted);
    let found = db.get_model_by_id(created.id).await.unwrap();
    assert!(found.is_none());
}

