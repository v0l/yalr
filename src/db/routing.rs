use super::{
    Database, NewRoutingConfig, NewRoutingConfigProvider, RoutingConfig, RoutingConfigProvider, UpdateRoutingConfig,
    UpdateRoutingConfigProvider,
};
use sqlx::QueryBuilder;

impl Database {
    pub async fn create_routing_config(&self, rc: NewRoutingConfig) -> Result<RoutingConfig, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO routing_config (name, strategy, health_check_enabled, health_check_interval_seconds, \
             health_check_timeout_seconds) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(rc.name)
        .bind(rc.strategy)
        .bind(rc.health_check_enabled)
        .bind(rc.health_check_interval_seconds)
        .bind(rc.health_check_timeout_seconds)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.get_routing_config(id).await?.ok_or(sqlx::Error::RowNotFound)
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

    pub async fn get_routing_config_by_name(&self, name: &str) -> Result<Option<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config WHERE name = ? ORDER BY id LIMIT 1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_routing_configs(&self) -> Result<Vec<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config ORDER BY id").fetch_all(&self.pool).await
    }

    pub async fn update_routing_config(
        &self,
        id: i64,
        updates: UpdateRoutingConfig,
    ) -> Result<RoutingConfig, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE routing_config SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(name) = updates.name {
            query.push(", name = ").push_bind(name);
        }
        if let Some(strategy) = updates.strategy {
            query.push(", strategy = ").push_bind(strategy);
        }
        if let Some(enabled) = updates.health_check_enabled {
            query.push(", health_check_enabled = ").push_bind(enabled);
        }
        if let Some(interval) = updates.health_check_interval_seconds {
            query.push(", health_check_interval_seconds = ").push_bind(interval);
        }
        if let Some(timeout) = updates.health_check_timeout_seconds {
            query.push(", health_check_timeout_seconds = ").push_bind(timeout);
        }
        query.push(" WHERE id = ").push_bind(id);
        query.build().execute(&self.pool).await?;
        self.get_routing_config(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_routing_config(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM routing_config WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_routing_config_provider(
        &self,
        rcp: NewRoutingConfigProvider,
    ) -> Result<RoutingConfigProvider, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO routing_config_providers (routing_config_id, provider_id, model, weight, is_active) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(rcp.routing_config_id)
        .bind(rcp.provider_id)
        .bind(rcp.model)
        .bind(rcp.weight)
        .bind(rcp.is_active)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.get_routing_config_provider(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn get_routing_config_provider(&self, id: i64) -> Result<Option<RoutingConfigProvider>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfigProvider>("SELECT * FROM routing_config_providers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_active_routing_config_providers(
        &self,
        routing_config_id: i64,
    ) -> Result<Vec<RoutingConfigProvider>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfigProvider>(
            "SELECT * FROM routing_config_providers WHERE routing_config_id = ? AND is_active = TRUE \
             ORDER BY weight DESC, id",
        )
        .bind(routing_config_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_routing_config_providers_for_config(
        &self,
        routing_config_id: i64,
    ) -> Result<Vec<RoutingConfigProvider>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfigProvider>(
            "SELECT * FROM routing_config_providers WHERE routing_config_id = ? ORDER BY weight DESC, id",
        )
        .bind(routing_config_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_routing_config_providers_for_provider(
        &self,
        provider_id: i64,
    ) -> Result<Vec<RoutingConfigProvider>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfigProvider>(
            "SELECT * FROM routing_config_providers WHERE provider_id = ? ORDER BY routing_config_id, id",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_routing_config_provider(
        &self,
        id: i64,
        updates: UpdateRoutingConfigProvider,
    ) -> Result<RoutingConfigProvider, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE routing_config_providers SET updated_at = CURRENT_TIMESTAMP(3)");
        query.push(", model = ").push_bind(updates.model);
        if let Some(weight) = updates.weight {
            query.push(", weight = ").push_bind(weight);
        }
        if let Some(is_active) = updates.is_active {
            query.push(", is_active = ").push_bind(is_active);
        }
        query.push(" WHERE id = ").push_bind(id);
        query.build().execute(&self.pool).await?;
        self.get_routing_config_provider(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_routing_config_provider(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM routing_config_providers WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }
}
