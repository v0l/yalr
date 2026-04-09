pub mod schema;

use sqlx::SqlitePool;
use std::sync::Arc;
use crate::db::schema::{CREATE_PROVIDERS_TABLE, CREATE_MODELS_TABLE, CREATE_MODEL_PROVIDERS_TABLE, CREATE_ROUTING_CONFIG_TABLE};

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct Model {
    pub id: i64,
    pub name: String,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct ModelProvider {
    pub id: i64,
    pub model_id: i64,
    pub provider_id: i64,
    pub weight: i32,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct RoutingConfig {
    pub id: i64,
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug)]
pub struct NewProvider<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct NewModel<'a> {
    pub name: &'a str,
    pub cost_per_1m_input: f64,
    pub cost_per_1m_output: f64,
}

#[derive(Clone, Debug)]
pub struct NewModelProvider {
    pub model_id: i64,
    pub provider_id: i64,
    pub weight: i32,
    pub is_active: bool,
}

#[derive(Clone, Debug)]
pub struct NewRoutingConfig {
    pub strategy: String,
    pub health_check_enabled: bool,
    pub health_check_interval_seconds: i32,
    pub health_check_timeout_seconds: i32,
}

#[derive(Clone, Debug)]
pub struct UpdateProvider<'a> {
    pub name: Option<&'a str>,
    pub slug: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub api_key: Option<Option<&'a str>>,
}

#[derive(Clone, Debug)]
pub struct UpdateModel<'a> {
    pub name: Option<&'a str>,
    pub cost_per_1m_input: Option<f64>,
    pub cost_per_1m_output: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct UpdateModelProvider {
    pub weight: Option<i32>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct UpdateRoutingConfig {
    pub strategy: Option<String>,
    pub health_check_enabled: Option<bool>,
    pub health_check_interval_seconds: Option<i32>,
    pub health_check_timeout_seconds: Option<i32>,
}

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(database_url).await?;
        Self::initialize_schema(&pool).await?;
        Ok(Self { pool })
    }

    async fn initialize_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
        sqlx::query(CREATE_PROVIDERS_TABLE).execute(pool).await?;
        sqlx::query(CREATE_MODELS_TABLE).execute(pool).await?;
        sqlx::query(CREATE_MODEL_PROVIDERS_TABLE).execute(pool).await?;
        sqlx::query(CREATE_ROUTING_CONFIG_TABLE).execute(pool).await?;
        Ok(())
    }

    pub async fn close(&self) -> Result<(), sqlx::Error> {
        self.pool.close().await;
        Ok(())
    }

    // Provider CRUD
    pub async fn create_provider(&self, provider: NewProvider<'_>) -> Result<Provider, sqlx::Error> {
        sqlx::query_as::<_, Provider>(
            "INSERT INTO providers (name, slug, base_url, api_key) VALUES (?, ?, ?, ?) RETURNING *"
        )
        .bind(provider.name)
        .bind(provider.slug)
        .bind(provider.base_url)
        .bind(provider.api_key)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_provider_by_id(&self, id: i64) -> Result<Option<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_provider_by_slug(&self, slug: &str) -> Result<Option<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_providers(&self) -> Result<Vec<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers ORDER BY name")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn update_provider(&self, id: i64, updates: UpdateProvider<'_>) -> Result<Provider, sqlx::Error> {
        let mut query = String::from("UPDATE providers SET updated_at = CURRENT_TIMESTAMP");
        
        if let Some(_name) = updates.name {
            query.push_str(", name = ?");
        }
        if let Some(_slug) = updates.slug {
            query.push_str(", slug = ?");
        }
        if let Some(_base_url) = updates.base_url {
            query.push_str(", base_url = ?");
        }
        if let Some(_api_key) = updates.api_key {
            query.push_str(", api_key = ?");
        }

        query.push_str(" WHERE id = ? RETURNING *");

        let mut query_builder = sqlx::query_as::<_, Provider>(&query);
        
        if let Some(name) = updates.name {
            query_builder = query_builder.bind(name);
        }
        if let Some(slug) = updates.slug {
            query_builder = query_builder.bind(slug);
        }
        if let Some(base_url) = updates.base_url {
            query_builder = query_builder.bind(base_url);
        }
        if let Some(api_key) = updates.api_key {
            query_builder = query_builder.bind(api_key);
        }
        
        query_builder.bind(id).fetch_one(&self.pool).await
    }

    pub async fn delete_provider(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM providers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // Model CRUD
    pub async fn create_model(&self, model: NewModel<'_>) -> Result<Model, sqlx::Error> {
        sqlx::query_as::<_, Model>(
            "INSERT INTO models (name, cost_per_1m_input, cost_per_1m_output) VALUES (?, ?, ?) RETURNING *"
        )
        .bind(model.name)
        .bind(model.cost_per_1m_input)
        .bind(model.cost_per_1m_output)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_model_by_id(&self, id: i64) -> Result<Option<Model>, sqlx::Error> {
        sqlx::query_as::<_, Model>("SELECT * FROM models WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_model_by_name(&self, name: &str) -> Result<Option<Model>, sqlx::Error> {
        sqlx::query_as::<_, Model>("SELECT * FROM models WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_models(&self) -> Result<Vec<Model>, sqlx::Error> {
        sqlx::query_as::<_, Model>("SELECT * FROM models ORDER BY name")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn update_model(&self, id: i64, updates: UpdateModel<'_>) -> Result<Model, sqlx::Error> {
        let mut query = String::from("UPDATE models SET updated_at = CURRENT_TIMESTAMP");
        
        if let Some(_name) = updates.name {
            query.push_str(", name = ?");
        }
        if let Some(_cost_per_1m_input) = updates.cost_per_1m_input {
            query.push_str(", cost_per_1m_input = ?");
        }
        if let Some(_cost_per_1m_output) = updates.cost_per_1m_output {
            query.push_str(", cost_per_1m_output = ?");
        }

        query.push_str(" WHERE id = ? RETURNING *");

        let mut query_builder = sqlx::query_as::<_, Model>(&query);
        
        if let Some(name) = updates.name {
            query_builder = query_builder.bind(name);
        }
        if let Some(cost_per_1m_input) = updates.cost_per_1m_input {
            query_builder = query_builder.bind(cost_per_1m_input);
        }
        if let Some(cost_per_1m_output) = updates.cost_per_1m_output {
            query_builder = query_builder.bind(cost_per_1m_output);
        }
        
        query_builder.bind(id).fetch_one(&self.pool).await
    }

    pub async fn delete_model(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM models WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // ModelProvider CRUD
    pub async fn create_model_provider(&self, mp: NewModelProvider) -> Result<ModelProvider, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>(
            "INSERT INTO model_providers (model_id, provider_id, weight, is_active) VALUES (?, ?, ?, ?) RETURNING *"
        )
        .bind(mp.model_id)
        .bind(mp.provider_id)
        .bind(mp.weight)
        .bind(mp.is_active)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_model_provider(&self, model_id: i64, provider_id: i64) -> Result<Option<ModelProvider>, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers WHERE model_id = ? AND provider_id = ?")
            .bind(model_id)
            .bind(provider_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_model_providers(&self) -> Result<Vec<ModelProvider>, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers ORDER BY model_id, provider_id")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn list_model_providers_for_model(&self, model_id: i64) -> Result<Vec<ModelProvider>, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers WHERE model_id = ? ORDER BY provider_id")
            .bind(model_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn list_model_providers_for_provider(&self, provider_id: i64) -> Result<Vec<ModelProvider>, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers WHERE provider_id = ? ORDER BY model_id")
            .bind(provider_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn update_model_provider(&self, model_id: i64, provider_id: i64, updates: UpdateModelProvider) -> Result<ModelProvider, sqlx::Error> {
        let mut query = String::from("UPDATE model_providers SET updated_at = CURRENT_TIMESTAMP");
        
        if let Some(_weight) = updates.weight {
            query.push_str(", weight = ?");
        }
        if let Some(_is_active) = updates.is_active {
            query.push_str(", is_active = ?");
        }

        query.push_str(" WHERE model_id = ? AND provider_id = ? RETURNING *");

        let mut query_builder = sqlx::query_as::<_, ModelProvider>(&query);
        
        if let Some(weight) = updates.weight {
            query_builder = query_builder.bind(weight);
        }
        if let Some(is_active) = updates.is_active {
            query_builder = query_builder.bind(is_active);
        }
        
        query_builder.bind(model_id).bind(provider_id).fetch_one(&self.pool).await
    }

    pub async fn delete_model_provider(&self, model_id: i64, provider_id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM model_providers WHERE model_id = ? AND provider_id = ?")
            .bind(model_id)
            .bind(provider_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    // RoutingConfig CRUD
    pub async fn create_routing_config(&self, rc: NewRoutingConfig) -> Result<RoutingConfig, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>(
            "INSERT INTO routing_config (strategy, health_check_enabled, health_check_interval_seconds, health_check_timeout_seconds) VALUES (?, ?, ?, ?) RETURNING *"
        )
        .bind(rc.strategy)
        .bind(rc.health_check_enabled)
        .bind(rc.health_check_interval_seconds)
        .bind(rc.health_check_timeout_seconds)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_routing_config(&self, id: i64) -> Result<Option<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_first_routing_config(&self) -> Result<Option<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config ORDER BY id LIMIT 1")
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_routing_configs(&self) -> Result<Vec<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config ORDER BY id")
            .fetch_all(&self.pool)
            .await
    }

    pub async fn update_routing_config(&self, id: i64, updates: UpdateRoutingConfig) -> Result<RoutingConfig, sqlx::Error> {
        let mut query = String::from("UPDATE routing_config SET updated_at = CURRENT_TIMESTAMP");
        
        if let Some(ref _strategy) = updates.strategy {
            query.push_str(", strategy = ?");
        }
        if let Some(_health_check_enabled) = updates.health_check_enabled {
            query.push_str(", health_check_enabled = ?");
        }
        if let Some(_health_check_interval_seconds) = updates.health_check_interval_seconds {
            query.push_str(", health_check_interval_seconds = ?");
        }
        if let Some(_health_check_timeout_seconds) = updates.health_check_timeout_seconds {
            query.push_str(", health_check_timeout_seconds = ?");
        }

        query.push_str(" WHERE id = ? RETURNING *");

        let mut query_builder = sqlx::query_as::<_, RoutingConfig>(&query);
        
        if let Some(strategy) = updates.strategy {
            query_builder = query_builder.bind(strategy);
        }
        if let Some(health_check_enabled) = updates.health_check_enabled {
            query_builder = query_builder.bind(health_check_enabled);
        }
        if let Some(health_check_interval_seconds) = updates.health_check_interval_seconds {
            query_builder = query_builder.bind(health_check_interval_seconds);
        }
        if let Some(health_check_timeout_seconds) = updates.health_check_timeout_seconds {
            query_builder = query_builder.bind(health_check_timeout_seconds);
        }
        
        query_builder.bind(id).fetch_one(&self.pool).await
    }

    pub async fn delete_routing_config(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM routing_config WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

pub type DbPool = Arc<SqlitePool>;

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_db() -> Database {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        Database::initialize_schema(&pool).await.unwrap();
        Database { pool }
    }

    // Provider Tests
    #[tokio::test]
    async fn test_create_provider() {
        let db = setup_test_db().await;
        let provider = NewProvider {
            name: "test-provider",
            slug: "test",
            base_url: "http://localhost:8080",
            api_key: Some("test-key"),
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
        }).await.unwrap();
        db.create_provider(NewProvider {
            name: "provider2",
            slug: "p2",
            base_url: "http://localhost:2",
            api_key: None,
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
        };
        let created = db.create_provider(provider).await.unwrap();
        let updated = db.update_provider(created.id, UpdateProvider {
            name: Some("updated-name"),
            slug: Some("updated-slug"),
            base_url: Some("http://localhost:9090"),
            api_key: Some(Some("new-key")),
        }).await.unwrap();
        assert_eq!(updated.name, "updated-name");
        assert_eq!(updated.slug, "updated-slug");
        assert_eq!(updated.base_url, "http://localhost:9090");
        assert_eq!(updated.api_key, Some("new-key".to_string()));
    }

    #[tokio::test]
    async fn test_update_provider_partial() {
        let db = setup_test_db().await;
        let provider = NewProvider {
            name: "test-provider",
            slug: "test",
            base_url: "http://localhost:8080",
            api_key: Some("test-key"),
        };
        let created = db.create_provider(provider).await.unwrap();
        let updated = db.update_provider(created.id, UpdateProvider {
            name: Some("updated-name"),
            slug: None,
            base_url: None,
            api_key: None,
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

    // ModelProvider Tests
    #[tokio::test]
    async fn test_create_model_provider() {
        let db = setup_test_db().await;
        let provider = db.create_provider(NewProvider {
            name: "test-provider",
            slug: "test",
            base_url: "http://localhost:8080",
            api_key: None,
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
        }).await.unwrap();
        let provider2 = db.create_provider(NewProvider {
            name: "provider2",
            slug: "p2",
            base_url: "http://localhost:2",
            api_key: None,
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

    // RoutingConfig Tests
    #[tokio::test]
    async fn test_create_routing_config() {
        let db = setup_test_db().await;
        let rc = db.create_routing_config(NewRoutingConfig {
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        assert_eq!(rc.strategy, "round_robin");
        assert!(rc.health_check_enabled);
        assert_eq!(rc.health_check_interval_seconds, 30);
        assert_eq!(rc.health_check_timeout_seconds, 5);
    }

    #[tokio::test]
    async fn test_get_routing_config() {
        let db = setup_test_db().await;
        let rc = db.create_routing_config(NewRoutingConfig {
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        let found = db.get_routing_config(rc.id).await.unwrap().unwrap();
        assert_eq!(found.id, rc.id);
    }

    #[tokio::test]
    async fn test_get_first_routing_config() {
        let db = setup_test_db().await;
        let rc = db.create_routing_config(NewRoutingConfig {
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        let found = db.get_first_routing_config().await.unwrap().unwrap();
        assert_eq!(found.id, rc.id);
    }

    #[tokio::test]
    async fn test_list_routing_configs() {
        let db = setup_test_db().await;
        db.create_routing_config(NewRoutingConfig {
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        db.create_routing_config(NewRoutingConfig {
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
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        let updated = db.update_routing_config(rc.id, UpdateRoutingConfig {
            strategy: Some("weighted".to_string()),
            health_check_enabled: Some(false),
            health_check_interval_seconds: Some(60),
            health_check_timeout_seconds: Some(10),
        }).await.unwrap();
        assert_eq!(updated.strategy, "weighted");
        assert!(!updated.health_check_enabled);
        assert_eq!(updated.health_check_interval_seconds, 60);
        assert_eq!(updated.health_check_timeout_seconds, 10);
    }

    #[tokio::test]
    async fn test_update_routing_config_partial() {
        let db = setup_test_db().await;
        let rc = db.create_routing_config(NewRoutingConfig {
            strategy: "round_robin".to_string(),
            health_check_enabled: true,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        }).await.unwrap();
        let updated = db.update_routing_config(rc.id, UpdateRoutingConfig {
            strategy: Some("weighted".to_string()),
            health_check_enabled: None,
            health_check_interval_seconds: None,
            health_check_timeout_seconds: None,
        }).await.unwrap();
        assert_eq!(updated.strategy, "weighted");
        assert!(updated.health_check_enabled);
        assert_eq!(updated.health_check_interval_seconds, 30);
    }

    #[tokio::test]
    async fn test_delete_routing_config() {
        let db = setup_test_db().await;
        let rc = db.create_routing_config(NewRoutingConfig {
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
}
