use async_trait::async_trait;
use std::sync::Arc;

use crate::providers::Provider;

pub mod round_robin;

#[derive(Clone)]
pub struct ProviderEntry {
    pub provider: Arc<dyn Provider>,
    pub model_override: Option<String>,
    pub weight: i32,
}

#[async_trait]
pub trait RoutingStrategy: Send + Sync {
    fn name(&self) -> &str;

    async fn select(&self, entries: &[ProviderEntry], key: &str) -> Option<usize>;
}
