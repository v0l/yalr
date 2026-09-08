use async_openai::types::models::Model;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Last known good model listing per provider slug.
///
/// `/v1/models` serves this instead of dialing a provider that the health
/// checker already knows is down, so one dead upstream can't add its timeout
/// to every client's listing request.
#[derive(Clone, Default)]
pub struct ModelListCache {
    inner: Arc<RwLock<HashMap<String, Vec<Model>>>>,
}

impl ModelListCache {
    pub async fn get(&self, provider_slug: &str) -> Option<Vec<Model>> {
        self.inner.read().await.get(provider_slug).cloned()
    }

    pub async fn put(&self, provider_slug: &str, models: Vec<Model>) {
        self.inner
            .write()
            .await
            .insert(provider_slug.to_string(), models);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        Model {
            id: id.to_string(),
            object: "model".to_string(),
            created: 0,
            owned_by: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn returns_none_until_populated() {
        let cache = ModelListCache::default();
        assert!(cache.get("p1").await.is_none());

        cache.put("p1", vec![model("a")]).await;
        let cached = cache.get("p1").await.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].id, "a");
    }

    #[tokio::test]
    async fn put_replaces_previous_listing() {
        let cache = ModelListCache::default();
        cache.put("p1", vec![model("a"), model("b")]).await;
        cache.put("p1", vec![model("c")]).await;

        let cached = cache.get("p1").await.unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].id, "c");
    }

    #[tokio::test]
    async fn entries_are_per_provider() {
        let cache = ModelListCache::default();
        cache.put("p1", vec![model("a")]).await;
        assert!(cache.get("p2").await.is_none());
    }
}
