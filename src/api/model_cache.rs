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

/// How long a provider's reported input modalities stay fresh. `get_runtime_info`
/// is a network round-trip (`/props` for llama.cpp), and a model's modalities do
/// not change while the model stays loaded, so this only needs to be short
/// enough to notice a model reloaded with a different projector.
const MODALITY_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Cached input modalities per `<provider-slug>/<model-id>`.
///
/// Bounded by the number of models across modality-reporting providers, which is
/// small: `Provider::reports_modalities` is false for every backend that would
/// otherwise hardcode `Text`. Entries expire so a reloaded model is re-probed.
#[derive(Clone, Default)]
pub struct ModalityCache {
    inner: Arc<RwLock<HashMap<String, (Vec<String>, std::time::Instant)>>>,
}

impl ModalityCache {
    pub async fn get(&self, key: &str) -> Option<Vec<String>> {
        let guard = self.inner.read().await;
        let (modalities, cached_at) = guard.get(key)?;
        (cached_at.elapsed() < MODALITY_CACHE_TTL).then(|| modalities.clone())
    }

    pub async fn put(&self, key: &str, modalities: Vec<String>) {
        self.inner
            .write()
            .await
            .insert(key.to_string(), (modalities, std::time::Instant::now()));
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

    #[tokio::test]
    async fn modality_cache_returns_none_until_populated() {
        let cache = ModalityCache::default();
        assert!(cache.get("p1/m").await.is_none());

        cache
            .put("p1/m", vec!["text".to_string(), "image".to_string()])
            .await;
        assert_eq!(cache.get("p1/m").await.unwrap().len(), 2);
        assert!(cache.get("p1/other").await.is_none());
    }
}
