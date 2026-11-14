use super::{Database, NewProvider, Provider};

impl Database {
    pub async fn create_owned_provider(&self, owner_user_id: i64, provider: NewProvider<'_>) -> Result<Provider, sqlx::Error> {
        let created = self.create_provider(provider).await?;
        sqlx::query("UPDATE providers SET owner_user_id = ? WHERE id = ?")
            .bind(owner_user_id)
            .bind(created.id)
            .execute(&self.pool)
            .await?;
        self.get_provider_by_id(created.id).await?.ok_or(sqlx::Error::RowNotFound)
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
}
