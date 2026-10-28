use crate::db::*;
use super::setup_test_db;

// ModelProvider Tests
#[tokio::test]
async fn test_create_model_provider() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let mp = db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    assert_eq!(mp.model_id, model.id);
    assert_eq!(mp.provider_id, provider.id);
    assert_eq!(mp.weight, 100);
    assert!(mp.is_active);
}

#[tokio::test]
async fn test_get_model_provider() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let mp = db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    let found = db.get_model_provider(model.id, provider.id).await.unwrap().unwrap();
    assert_eq!(found.id, mp.id);
}

#[tokio::test]
async fn test_list_model_providers() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model1 = db.create_model(NewModel {
        name: "model1",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let model2 = db.create_model(NewModel {
        name: "model2",
        cost_per_1m_input: 0.03,
        cost_per_1m_output: 0.04,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model1.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model2.id,
        provider_id: provider.id,
        weight: 50,
        is_active: false,
    }).await.unwrap();
    let mps = db.list_model_providers().await.unwrap();
    assert_eq!(mps.len(), 2);
}

#[tokio::test]
async fn test_list_model_providers_for_model() {
    let db = setup_test_db().await;
    let provider1 = db.create_provider(NewProvider {
        name: "provider1",
        slug: "p1",
        base_url: "http://localhost:1",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let provider2 = db.create_provider(NewProvider {
        name: "provider2",
        slug: "p2",
        base_url: "http://localhost:2",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider1.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider2.id,
        weight: 50,
        is_active: true,
    }).await.unwrap();
    let mps = db.list_model_providers_for_model(model.id).await.unwrap();
    assert_eq!(mps.len(), 2);
}

#[tokio::test]
async fn test_list_model_providers_for_provider() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model1 = db.create_model(NewModel {
        name: "model1",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let model2 = db.create_model(NewModel {
        name: "model2",
        cost_per_1m_input: 0.03,
        cost_per_1m_output: 0.04,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model1.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_model_provider(NewModelProvider {
        model_id: model2.id,
        provider_id: provider.id,
        weight: 50,
        is_active: true,
    }).await.unwrap();
    let mps = db.list_model_providers_for_provider(provider.id).await.unwrap();
    assert_eq!(mps.len(), 2);
}

#[tokio::test]
async fn test_update_model_provider() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let _mp = db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    let updated = db.update_model_provider(model.id, provider.id, UpdateModelProvider {
        weight: Some(200),
        is_active: Some(false),
    }).await.unwrap();
    assert_eq!(updated.weight, 200);
    assert!(!updated.is_active);
}

#[tokio::test]
async fn test_update_model_provider_partial() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let _mp = db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    let updated = db.update_model_provider(model.id, provider.id, UpdateModelProvider {
        weight: Some(200),
        is_active: None,
    }).await.unwrap();
    assert_eq!(updated.weight, 200);
    assert!(updated.is_active);
}

#[tokio::test]
async fn test_delete_model_provider() {
    let db = setup_test_db().await;
    let provider = db.create_provider(NewProvider {
        name: "test-provider",
        slug: "test",
        base_url: "http://localhost:8080",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    let model = db.create_model(NewModel {
        name: "test-model",
        cost_per_1m_input: 0.01,
        cost_per_1m_output: 0.02,
    }).await.unwrap();
    let _mp = db.create_model_provider(NewModelProvider {
        model_id: model.id,
        provider_id: provider.id,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    let deleted = db.delete_model_provider(model.id, provider.id).await.unwrap();
    assert!(deleted);
    let found = db.get_model_provider(model.id, provider.id).await.unwrap();
    assert!(found.is_none());
}

