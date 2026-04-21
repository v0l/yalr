use std::sync::Arc;
use crate::config::AppConfig;
use crate::db::Database;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::auth::admin::SessionStore;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub metrics_emitter: MetricsEmitter,
    pub metrics_store: MetricsStore,
    pub session_store: Arc<SessionStore>,
    pub db: Arc<Database>,
}
