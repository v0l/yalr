use super::Database;
use serde_json::{Map, Value};

pub type ExtraBody = Map<String, Value>;

pub fn validate_extra_body(value: Value) -> Result<Option<ExtraBody>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) if map.is_empty() => Ok(None),
        Value::Object(map) => Ok(Some(map)),
        _ => Err("extra_body must be a JSON object".to_string()),
    }
}

pub fn parse_stored_extra_body(raw: Option<&str>) -> Option<ExtraBody> {
    match serde_json::from_str::<Value>(raw?) {
        Ok(Value::Object(map)) if !map.is_empty() => Some(map),
        _ => None,
    }
}

pub fn merge_extra_body(base: &mut ExtraBody, over: &ExtraBody) {
    for (key, value) in over {
        base.insert(key.clone(), value.clone());
    }
}

impl Database {
    pub async fn set_provider_extra_body(
        &self,
        id: i64,
        extra_body: Option<&ExtraBody>,
    ) -> Result<super::Provider, sqlx::Error> {
        let encoded = extra_body.and_then(|map| serde_json::to_string(map).ok());
        sqlx::query("UPDATE providers SET extra_body = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?")
            .bind(encoded)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_provider_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn set_routing_config_provider_extra_body(
        &self,
        id: i64,
        extra_body: Option<&ExtraBody>,
    ) -> Result<super::RoutingConfigProvider, sqlx::Error> {
        let encoded = extra_body.and_then(|map| serde_json::to_string(map).ok());
        sqlx::query(
            "UPDATE routing_config_providers SET extra_body = ?, updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?",
        )
        .bind(encoded)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_routing_config_provider(id).await?.ok_or(sqlx::Error::RowNotFound)
    }
}

impl super::Provider {
    pub fn extra_body_map(&self) -> Option<ExtraBody> {
        parse_stored_extra_body(self.extra_body.as_deref())
    }
}

impl super::RoutingConfigProvider {
    pub fn extra_body_map(&self) -> Option<ExtraBody> {
        parse_stored_extra_body(self.extra_body.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{NewProvider, NewRoutingConfig, NewRoutingConfigProvider};
    use serde_json::json;

    #[test]
    fn rejects_non_objects() {
        assert!(validate_extra_body(json!([1])).is_err());
        assert!(validate_extra_body(json!("x")).is_err());
        assert!(validate_extra_body(json!(3)).is_err());
    }

    #[test]
    fn null_and_empty_object_clear() {
        assert_eq!(validate_extra_body(Value::Null), Ok(None));
        assert_eq!(validate_extra_body(json!({})), Ok(None));
        assert!(validate_extra_body(json!({"a": 1})).unwrap().is_some());
    }

    #[test]
    fn merge_replaces_top_level_keys_whole() {
        let mut base = json!({"provider": {"order": ["a"], "only": ["a"]}, "x": 1}).as_object().unwrap().clone();
        let over = json!({"provider": {"order": ["b"]}}).as_object().unwrap().clone();
        merge_extra_body(&mut base, &over);
        assert_eq!(Value::Object(base), json!({"provider": {"order": ["b"]}, "x": 1}));
    }

    #[tokio::test]
    async fn extra_body_round_trips_on_provider_and_routing_entry() {
        let db = Database::test().await;
        let provider = db
            .create_provider(NewProvider {
                name: "p",
                slug: "p",
                base_url: "http://x",
                api_key: None,
                provider_type: None,
            })
            .await
            .unwrap();
        assert!(provider.extra_body_map().is_none());
        let body = json!({"provider": {"order": ["x"], "allow_fallbacks": false}}).as_object().unwrap().clone();
        let updated = db.set_provider_extra_body(provider.id, Some(&body)).await.unwrap();
        assert_eq!(updated.extra_body_map(), Some(body.clone()));
        let cleared = db.set_provider_extra_body(provider.id, None).await.unwrap();
        assert!(cleared.extra_body_map().is_none());

        let config = db
            .create_routing_config(NewRoutingConfig {
                name: "m".to_string(),
                strategy: "round_robin".to_string(),
                health_check_enabled: false,
                health_check_interval_seconds: 30,
                health_check_timeout_seconds: 5,
            })
            .await
            .unwrap();
        let entry = db
            .create_routing_config_provider(NewRoutingConfigProvider {
                routing_config_id: config.id,
                provider_id: provider.id,
                model: None,
                weight: 1,
                is_active: true,
            })
            .await
            .unwrap();
        let entry = db.set_routing_config_provider_extra_body(entry.id, Some(&body)).await.unwrap();
        assert_eq!(entry.extra_body_map(), Some(body));
    }
}
