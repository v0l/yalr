use std::sync::RwLock;

use super::config::PayoutsConfig;

static SETTINGS: RwLock<Option<PayoutsConfig>> = RwLock::new(None);

pub fn set(config: PayoutsConfig) {
    if let Ok(mut guard) = SETTINGS.write() {
        *guard = Some(config);
    }
}

pub fn get() -> PayoutsConfig {
    SETTINGS.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}
