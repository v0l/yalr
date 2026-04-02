use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::providers::Provider;

pub mod round_robin;

pub type ProviderList = Vec<Arc<dyn Provider>>;

#[async_trait]
pub trait RoutingStrategy: Send + Sync {
    fn name(&self) -> &str;

    async fn select_provider(
        &self,
        providers: &[Arc<dyn Provider>],
        model: &str,
    ) -> Option<Arc<dyn Provider>>;

    async fn select_provider_by_slug(
        &self,
        providers: &[Arc<dyn Provider>],
        slug_prefix: &str,
    ) -> Option<Arc<dyn Provider>> {
        providers
            .iter()
            .find(|p| p.slug().starts_with(slug_prefix))
            .cloned()
    }
}

#[derive(Clone)]
pub struct RoutingEngine {
    strategy: Arc<dyn RoutingStrategy>,
    providers: Arc<RwLock<HashMap<String, Arc<dyn Provider>>>>,
}

impl RoutingEngine {
    pub fn new(strategy: Arc<dyn RoutingStrategy>) -> Self {
        Self {
            strategy,
            providers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_provider(&self, provider: Arc<dyn Provider>) {
        let mut providers = self.providers.write().await;
        providers.insert(provider.name().to_string(), provider);
    }

    pub async fn remove_provider(&self, name: &str) -> Option<Arc<dyn Provider>> {
        let mut providers = self.providers.write().await;
        providers.remove(name)
    }

    pub async fn get_providers(&self) -> Vec<Arc<dyn Provider>> {
        let providers = self.providers.read().await;
        providers.values().cloned().collect()
    }

    pub async fn route(&self, model: &str) -> Option<Arc<dyn Provider>> {
        let providers = self.get_providers().await;
        let provider_count = providers.len();

        if let Some((slug_prefix, _)) = model.split_once('/') {
            let result = self
                .strategy
                .select_provider_by_slug(&providers, slug_prefix)
                .await;
            if let Some(ref provider) = result {
                tracing::info!(
                    model = model,
                    slug_prefix = slug_prefix,
                    selected_provider = provider.name(),
                    strategy = self.strategy.name(),
                    "Routed by slug"
                );
            } else {
                tracing::warn!(
                    model = model,
                    slug_prefix = slug_prefix,
                    "No provider found for slug prefix"
                );
            }
            return result;
        }

        let result = self.strategy.select_provider(&providers, model).await;
        if let Some(ref provider) = result {
            tracing::info!(
                model = model,
                selected_provider = provider.name(),
                strategy = self.strategy.name(),
                provider_count = provider_count,
                "Provider routed"
            );
        } else {
            tracing::warn!(
                model = model,
                provider_count = provider_count,
                "No provider found for model"
            );
        }
        result
    }

    pub async fn route_by_slug(&self, slug_prefix: &str) -> Option<Arc<dyn Provider>> {
        let providers = self.get_providers().await;
        let result = self.strategy
            .select_provider_by_slug(&providers, slug_prefix)
            .await;
        
        if let Some(ref provider) = result {
            tracing::info!(
                slug_prefix = slug_prefix,
                selected_provider = provider.name(),
                strategy = self.strategy.name(),
                "Routed by slug"
            );
        } else {
            tracing::warn!(
                slug_prefix = slug_prefix,
                "No provider found for slug prefix"
            );
        }
        result
    }
}
