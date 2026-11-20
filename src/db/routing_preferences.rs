use super::Database;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingPreferences {
    pub default: Option<String>,
    pub models: Vec<(String, String)>,
}

impl Database {
    pub async fn routing_preferences(&self, user_id: i64) -> Result<RoutingPreferences, sqlx::Error> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT model_key, preference FROM user_routing_preferences WHERE user_id = ? ORDER BY model_key")
                .bind(user_id)
                .fetch_all(&self.pool)
                .await?;
        let mut out = RoutingPreferences::default();
        for (key, preference) in rows {
            if key.is_empty() {
                out.default = Some(preference);
            } else {
                out.models.push((key, preference));
            }
        }
        Ok(out)
    }

    pub async fn replace_routing_preferences(&self, user_id: i64, prefs: &RoutingPreferences) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM user_routing_preferences WHERE user_id = ?").bind(user_id).execute(&mut *tx).await?;
        let rows = prefs
            .default
            .iter()
            .map(|p| (String::new(), p.clone()))
            .chain(prefs.models.iter().cloned());
        for (key, preference) in rows {
            sqlx::query("INSERT INTO user_routing_preferences (user_id, model_key, preference) VALUES (?, ?, ?)")
                .bind(user_id)
                .bind(key)
                .bind(preference)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    pub async fn set_provider_max_concurrency(&self, provider_id: i64, limit: Option<i32>) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE providers SET max_concurrency = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?")
            .bind(limit)
            .bind(provider_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_provider_tuning(
        &self,
        provider_id: i64,
        ttft_deadline_ms: Option<i32>,
        reasoning_effort_map: Option<&crate::providers::effort::EffortMap>,
    ) -> Result<(), sqlx::Error> {
        let encoded = reasoning_effort_map.and_then(|m| serde_json::to_string(m).ok());
        sqlx::query(
            "UPDATE providers SET ttft_deadline_ms = ?, reasoning_effort_map = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?",
        )
        .bind(ttft_deadline_ms)
        .bind(encoded)
        .bind(provider_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewProvider;

    #[tokio::test]
    async fn preferences_round_trip_per_user() {
        let db = Database::test().await;
        let alice = db.add_test_user("alice", false).await;
        let bob = db.add_test_user("bob", false).await;
        assert_eq!(db.routing_preferences(alice.id).await.unwrap(), RoutingPreferences::default());
        let prefs = RoutingPreferences {
            default: Some("cheapest".into()),
            models: vec![("acme/m-1".into(), "fastest".into())],
        };
        db.replace_routing_preferences(alice.id, &prefs).await.unwrap();
        assert_eq!(db.routing_preferences(alice.id).await.unwrap(), prefs);
        assert_eq!(db.routing_preferences(bob.id).await.unwrap(), RoutingPreferences::default());
        db.replace_routing_preferences(alice.id, &RoutingPreferences::default()).await.unwrap();
        assert_eq!(db.routing_preferences(alice.id).await.unwrap(), RoutingPreferences::default());
    }

    #[tokio::test]
    async fn max_concurrency_is_stored_and_cleared() {
        let db = Database::test().await;
        let p = db
            .create_provider(NewProvider { name: "p", slug: "p", base_url: "http://x", api_key: None, provider_type: None })
            .await
            .unwrap();
        assert_eq!(p.max_concurrency, None);
        db.set_provider_max_concurrency(p.id, Some(4)).await.unwrap();
        assert_eq!(db.get_provider_by_id(p.id).await.unwrap().unwrap().max_concurrency, Some(4));
        db.set_provider_max_concurrency(p.id, None).await.unwrap();
        assert_eq!(db.get_provider_by_id(p.id).await.unwrap().unwrap().max_concurrency, None);
    }
}
