use crate::db::*;
use super::setup_test_db;

// RoutingConfig Tests
#[tokio::test]
async fn test_create_routing_config() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    assert_eq!(rc.name, "test-config");
    assert_eq!(rc.strategy, "round_robin");
    assert!(rc.health_check_enabled);
    assert_eq!(rc.health_check_interval_seconds, 30);
    assert_eq!(rc.health_check_timeout_seconds, 5);
}

#[tokio::test]
async fn test_get_routing_config() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let found = db.get_routing_config(rc.id).await.unwrap().unwrap();
    assert_eq!(found.id, rc.id);
    assert_eq!(found.name, "test-config");
}

#[tokio::test]
async fn test_get_first_routing_config() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let found = db.get_first_routing_config().await.unwrap().unwrap();
    assert_eq!(found.id, rc.id);
    assert_eq!(found.name, "test-config");
}

#[tokio::test]
async fn test_list_routing_configs() {
    let db = setup_test_db().await;
    db.create_routing_config(NewRoutingConfig {
        name: "config1".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    db.create_routing_config(NewRoutingConfig {
        name: "config2".to_string(),
        strategy: "weighted".to_string(),
        health_check_enabled: false,
        health_check_interval_seconds: 60,
        health_check_timeout_seconds: 10,
    }).await.unwrap();
    let configs = db.list_routing_configs().await.unwrap();
    assert_eq!(configs.len(), 2);
}

#[tokio::test]
async fn test_update_routing_config() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let updated = db.update_routing_config(rc.id, UpdateRoutingConfig {
        name: Some("updated-config".to_string()),
        strategy: Some("weighted".to_string()),
        health_check_enabled: Some(false),
        health_check_interval_seconds: Some(60),
        health_check_timeout_seconds: Some(10),
    }).await.unwrap();
    assert_eq!(updated.name, "updated-config");
    assert_eq!(updated.strategy, "weighted");
    assert!(!updated.health_check_enabled);
    assert_eq!(updated.health_check_interval_seconds, 60);
    assert_eq!(updated.health_check_timeout_seconds, 10);
}

#[tokio::test]
async fn test_update_routing_config_partial() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let updated = db.update_routing_config(rc.id, UpdateRoutingConfig {
        name: None,
        strategy: Some("weighted".to_string()),
        health_check_enabled: None,
        health_check_interval_seconds: None,
        health_check_timeout_seconds: None,
    }).await.unwrap();
    assert_eq!(updated.name, "test-config");
    assert_eq!(updated.strategy, "weighted");
    assert!(updated.health_check_enabled);
    assert_eq!(updated.health_check_interval_seconds, 30);
}

#[tokio::test]
async fn test_delete_routing_config() {
    let db = setup_test_db().await;
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "test-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let deleted = db.delete_routing_config(rc.id).await.unwrap();
    assert!(deleted);
    let found = db.get_routing_config(rc.id).await.unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn test_routing_config_with_multiple_providers() {
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
    let provider3 = db.create_provider(NewProvider {
        name: "provider3",
        slug: "p3",
        base_url: "http://localhost:3",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "multi-provider-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc.id,
        provider_id: provider1.id,
        model: None,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc.id,
        provider_id: provider2.id,
        model: None,
        weight: 90,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc.id,
        provider_id: provider3.id,
        model: None,
        weight: 80,
        is_active: true,
    }).await.unwrap();
    
    let providers = db.list_active_routing_config_providers(rc.id).await.unwrap();
    assert_eq!(providers.len(), 3);
    
    let all_providers = db.list_providers().await.unwrap();
    assert_eq!(all_providers.len(), 3);
}

#[tokio::test]
async fn test_routing_config_provider_filtering_by_active() {
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
    
    let rc = db.create_routing_config(NewRoutingConfig {
        name: "active-filtering-config".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc.id,
        provider_id: provider1.id,
        model: None,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc.id,
        provider_id: provider2.id,
        model: None,
        weight: 90,
        is_active: false,
    }).await.unwrap();
    
    let active_providers = db.list_active_routing_config_providers(rc.id).await.unwrap();
    assert_eq!(active_providers.len(), 1);
    assert_eq!(active_providers[0].provider_id, provider1.id);
}

#[tokio::test]
async fn test_multiple_routing_configs_with_different_providers() {
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
    let provider3 = db.create_provider(NewProvider {
        name: "provider3",
        slug: "p3",
        base_url: "http://localhost:3",
        api_key: None,
        provider_type: None,
    }).await.unwrap();
    
    let rc1 = db.create_routing_config(NewRoutingConfig {
        name: "config1".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    let rc2 = db.create_routing_config(NewRoutingConfig {
        name: "config2".to_string(),
        strategy: "round_robin".to_string(),
        health_check_enabled: true,
        health_check_interval_seconds: 30,
        health_check_timeout_seconds: 5,
    }).await.unwrap();
    
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc1.id,
        provider_id: provider1.id,
        model: None,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc1.id,
        provider_id: provider2.id,
        model: None,
        weight: 90,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc2.id,
        provider_id: provider2.id,
        model: None,
        weight: 100,
        is_active: true,
    }).await.unwrap();
    db.create_routing_config_provider(NewRoutingConfigProvider {
        routing_config_id: rc2.id,
        provider_id: provider3.id,
        model: None,
        weight: 90,
        is_active: true,
    }).await.unwrap();
    
    let rc1_providers = db.list_active_routing_config_providers(rc1.id).await.unwrap();
    let rc2_providers = db.list_active_routing_config_providers(rc2.id).await.unwrap();
    
    assert_eq!(rc1_providers.len(), 2);
    assert_eq!(rc2_providers.len(), 2);
    
    assert!(rc1_providers.iter().any(|p| p.provider_id == provider1.id));
    assert!(rc1_providers.iter().any(|p| p.provider_id == provider2.id));
    assert!(rc2_providers.iter().any(|p| p.provider_id == provider2.id));
    assert!(rc2_providers.iter().any(|p| p.provider_id == provider3.id));
}

