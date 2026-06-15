use crate::auth::admin::SessionStore;
use crate::config::AppConfig;
use crate::db::Database;
use crate::metrics::{MetricsEmitter, MetricsStore};
use crate::payments::PaymentsState;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub metrics_emitter: MetricsEmitter,
    pub metrics_store: std::sync::Arc<MetricsStore>,
    pub session_store: Arc<SessionStore>,
    pub db: Arc<Database>,
    pub payments_state: Option<Arc<PaymentsState>>,
    /// In-flight OAuth logins awaiting code submission, keyed by `state`.
    pub oauth_pending: crate::oauth::PendingStore,
}
