use super::Database;

impl Database {
    pub async fn nostr_secret_key(&self) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT secret_key FROM nostr_identity WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn insert_nostr_secret_key(&self, secret_key: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT IGNORE INTO nostr_identity (id, secret_key) VALUES (1, ?)")
            .bind(secret_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_key_wins() {
        let db = Database::test().await;
        assert_eq!(db.nostr_secret_key().await.unwrap(), None);
        db.insert_nostr_secret_key("first").await.unwrap();
        db.insert_nostr_secret_key("second").await.unwrap();
        assert_eq!(db.nostr_secret_key().await.unwrap().as_deref(), Some("first"));
    }
}
