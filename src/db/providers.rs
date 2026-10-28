use super::{
    Database, Model, ModelProvider, NewModel, NewModelProvider, NewProvider, OAuthCredentials, Provider, ProviderType,
    UpdateModel, UpdateModelProvider, UpdateProvider,
};
use sqlx::QueryBuilder;

impl Database {
    pub async fn create_provider(&self, provider: NewProvider<'_>) -> Result<Provider, sqlx::Error> {
        let provider_type = provider.provider_type.unwrap_or(ProviderType::OpenAi);
        let id = sqlx::query(
            "INSERT INTO providers (name, slug, base_url, api_key, provider_type) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(provider.name)
        .bind(provider.slug)
        .bind(provider.base_url)
        .bind(provider.api_key)
        .bind(provider_type as u16)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.provider_by_id(id).await
    }

    pub async fn get_provider_by_id(&self, id: i64) -> Result<Option<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    async fn provider_by_id(&self, id: i64) -> Result<Provider, sqlx::Error> {
        self.get_provider_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
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
        let mut query = QueryBuilder::new("UPDATE providers SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(name) = updates.name {
            query.push(", name = ").push_bind(name);
        }
        if let Some(slug) = updates.slug {
            query.push(", slug = ").push_bind(slug);
        }
        if let Some(base_url) = updates.base_url {
            query.push(", base_url = ").push_bind(base_url);
        }
        if let Some(api_key) = updates.api_key {
            query.push(", api_key = ").push_bind(api_key);
        }
        if let Some(provider_type) = updates.provider_type {
            query.push(", provider_type = ").push_bind(provider_type as u16);
        }
        query.push(" WHERE id = ").push_bind(id);
        query.build().execute(&self.pool).await?;
        self.provider_by_id(id).await
    }

    pub async fn delete_provider(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM providers WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_provider_by_slug(&self, slug: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM providers WHERE slug = ?").bind(slug).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_provider_by_name(&self, name: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM providers WHERE name = ?").bind(name).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn set_provider_client_version(
        &self,
        id: i64,
        client_version: Option<&str>,
    ) -> Result<Provider, sqlx::Error> {
        sqlx::query("UPDATE providers SET client_version = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?")
            .bind(client_version)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.provider_by_id(id).await
    }

    pub async fn update_provider_oauth(&self, id: i64, creds: &OAuthCredentials) -> Result<Provider, sqlx::Error> {
        sqlx::query(
            "UPDATE providers SET oauth_access_token = ?, oauth_refresh_token = ?, \
             oauth_expires_at = ?, oauth_account_id = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?",
        )
        .bind(&creds.access_token)
        .bind(&creds.refresh_token)
        .bind(creds.expires_at)
        .bind(creds.account_id.as_deref())
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.provider_by_id(id).await
    }

    pub async fn create_model(&self, model: NewModel<'_>) -> Result<Model, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO models (name, cost_per_1m_input, cost_per_1m_output) VALUES (?, ?, ?)",
        )
        .bind(model.name)
        .bind(model.cost_per_1m_input)
        .bind(model.cost_per_1m_output)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.get_model_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
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
        sqlx::query_as::<_, Model>("SELECT * FROM models ORDER BY name").fetch_all(&self.pool).await
    }

    pub async fn update_model(&self, id: i64, updates: UpdateModel<'_>) -> Result<Model, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE models SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(name) = updates.name {
            query.push(", name = ").push_bind(name);
        }
        if let Some(cost) = updates.cost_per_1m_input {
            query.push(", cost_per_1m_input = ").push_bind(cost);
        }
        if let Some(cost) = updates.cost_per_1m_output {
            query.push(", cost_per_1m_output = ").push_bind(cost);
        }
        query.push(" WHERE id = ").push_bind(id);
        query.build().execute(&self.pool).await?;
        self.get_model_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_model(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM models WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_model_provider(&self, mp: NewModelProvider) -> Result<ModelProvider, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO model_providers (model_id, provider_id, weight, is_active) VALUES (?, ?, ?, ?)",
        )
        .bind(mp.model_id)
        .bind(mp.provider_id)
        .bind(mp.weight)
        .bind(mp.is_active)
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.model_provider_by_id(id).await
    }

    async fn model_provider_by_id(&self, id: i64) -> Result<ModelProvider, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_model_provider(
        &self,
        model_id: i64,
        provider_id: i64,
    ) -> Result<Option<ModelProvider>, sqlx::Error> {
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

    pub async fn list_model_providers_for_provider(
        &self,
        provider_id: i64,
    ) -> Result<Vec<ModelProvider>, sqlx::Error> {
        sqlx::query_as::<_, ModelProvider>("SELECT * FROM model_providers WHERE provider_id = ? ORDER BY model_id")
            .bind(provider_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn update_model_provider(
        &self,
        model_id: i64,
        provider_id: i64,
        updates: UpdateModelProvider,
    ) -> Result<ModelProvider, sqlx::Error> {
        let mut query = QueryBuilder::new("UPDATE model_providers SET updated_at = CURRENT_TIMESTAMP(3)");
        if let Some(weight) = updates.weight {
            query.push(", weight = ").push_bind(weight);
        }
        if let Some(is_active) = updates.is_active {
            query.push(", is_active = ").push_bind(is_active);
        }
        query.push(" WHERE model_id = ").push_bind(model_id).push(" AND provider_id = ").push_bind(provider_id);
        query.build().execute(&self.pool).await?;
        self.get_model_provider(model_id, provider_id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn delete_model_provider(&self, model_id: i64, provider_id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM model_providers WHERE model_id = ? AND provider_id = ?")
            .bind(model_id)
            .bind(provider_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
