use crate::config::RequestLogConfig;
use std::sync::RwLock;

static SETTINGS: RwLock<Option<RequestLogConfig>> = RwLock::new(None);

pub fn set(config: RequestLogConfig) {
    if let Ok(mut guard) = SETTINGS.write() {
        *guard = Some(config);
    }
}

pub fn get() -> RequestLogConfig {
    SETTINGS
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_default()
}
