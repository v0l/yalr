use super::{Database, NewProvider, NewRoutingConfig, Provider, RoutingConfig};

impl Database {
    pub async fn create_owned_provider(&self, owner_user_id: i64, provider: NewProvider<'_>) -> Result<Provider, sqlx::Error> {
        let created = self.create_provider(provider).await?;
        sqlx::query("UPDATE providers SET owner_user_id = ?, status = 'pending' WHERE id = ?")
            .bind(owner_user_id)
            .bind(created.id)
            .execute(&self.pool)
            .await?;
        self.get_provider_by_id(created.id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn set_provider_owner(&self, provider_id: i64, owner_user_id: Option<i64>) -> Result<Provider, sqlx::Error> {
        sqlx::query("UPDATE providers SET owner_user_id = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?")
            .bind(owner_user_id)
            .bind(provider_id)
            .execute(&self.pool)
            .await?;
        self.get_provider_by_id(provider_id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn list_providers_owned_by(&self, owner_user_id: i64) -> Result<Vec<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE owner_user_id = ? ORDER BY name")
            .bind(owner_user_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn provider_owner_username(&self, owner_user_id: i64) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT COALESCE(username, external_id) FROM users WHERE id = ?")
            .bind(owner_user_id)
            .fetch_optional(&self.pool)
            .await
            .map(|name: Option<Option<String>>| name.flatten())
    }

    pub async fn create_owned_routing_config(
        &self,
        owner_user_id: Option<i64>,
        config: NewRoutingConfig,
    ) -> Result<RoutingConfig, sqlx::Error> {
        let created = self.create_routing_config(config).await?;
        if owner_user_id.is_none() {
            return Ok(created);
        }
        sqlx::query("UPDATE routing_config SET owner_user_id = ? WHERE id = ?")
            .bind(owner_user_id)
            .bind(created.id)
            .execute(&self.pool)
            .await?;
        self.get_routing_config(created.id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn list_routing_configs_owned_by(&self, owner_user_id: i64) -> Result<Vec<RoutingConfig>, sqlx::Error> {
        sqlx::query_as::<_, RoutingConfig>("SELECT * FROM routing_config WHERE owner_user_id = ? ORDER BY id")
            .bind(owner_user_id)
            .fetch_all(&self.pool)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn owned_providers_are_listed_per_owner() {
        let db = Database::test().await;
        let alice = db.add_test_user("alice", false).await;
        let bob = db.add_test_user("bob", false).await;
        let new = |slug: &'static str| NewProvider {
            name: slug,
            slug,
            base_url: "https://example.com/v1",
            api_key: Some("sk-secret"),
            provider_type: None,
        };
        let a = db.create_owned_provider(alice.id, new("a-one")).await.unwrap();
        db.create_owned_provider(bob.id, new("b-one")).await.unwrap();
        db.create_provider(new("platform")).await.unwrap();
        assert_eq!(a.owner_user_id, Some(alice.id));
        let mine = db.list_providers_owned_by(alice.id).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].slug, "a-one");
        assert!(db.get_provider_by_slug("platform").await.unwrap().unwrap().owner_user_id.is_none());
    }

    #[tokio::test]
    async fn an_existing_provider_can_be_linked_and_unlinked() {
        let db = Database::test().await;
        let alice = db.add_test_user("alice", false).await;
        let platform = db
            .create_provider(NewProvider {
                name: "platform",
                slug: "platform",
                base_url: "https://example.com/v1",
                api_key: None,
                provider_type: None,
            })
            .await
            .unwrap();
        let linked = db.set_provider_owner(platform.id, Some(alice.id)).await.unwrap();
        assert_eq!(linked.owner_user_id, Some(alice.id));
        assert_eq!(db.list_providers_owned_by(alice.id).await.unwrap().len(), 1);
        let unlinked = db.set_provider_owner(platform.id, None).await.unwrap();
        assert_eq!(unlinked.owner_user_id, None);
        assert!(db.list_providers_owned_by(alice.id).await.unwrap().is_empty());
    }
}
