use std::sync::RwLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VerificationConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval_hours")]
    pub provider_verify_interval_hours: u64,
    #[serde(default = "default_probe_max_tokens")]
    pub probe_max_tokens: u32,
    #[serde(default = "default_min_prefix_tokens")]
    pub min_prefix_tokens: u32,
    #[serde(default = "default_max_logprob_gap")]
    pub max_logprob_gap: f32,
    #[serde(default = "default_random_probes")]
    pub random_probes: usize,
    #[serde(default = "default_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default)]
    pub request_extras: std::collections::BTreeMap<String, serde_json::Value>,
}

fn default_true() -> bool {
    true
}

fn default_interval_hours() -> u64 {
    24
}

fn default_probe_max_tokens() -> u32 {
    32
}

fn default_min_prefix_tokens() -> u32 {
    16
}

fn default_max_logprob_gap() -> f32 {
    0.5
}

fn default_random_probes() -> usize {
    4
}

fn default_timeout_secs() -> u64 {
    90
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider_verify_interval_hours: default_interval_hours(),
            probe_max_tokens: default_probe_max_tokens(),
            min_prefix_tokens: default_min_prefix_tokens(),
            max_logprob_gap: default_max_logprob_gap(),
            random_probes: default_random_probes(),
            request_timeout_secs: default_timeout_secs(),
            request_extras: Default::default(),
        }
    }
}

static BACKGROUND: RwLock<bool> = RwLock::new(false);
static SETTINGS: RwLock<Option<VerificationConfig>> = RwLock::new(None);

pub fn set(config: VerificationConfig) {
    if let Ok(mut guard) = SETTINGS.write() {
        *guard = Some(config);
    }
}

pub fn get() -> VerificationConfig {
    SETTINGS.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}

pub fn enable_background() {
    if let Ok(mut flag) = BACKGROUND.write() {
        *flag = true;
    }
}

pub fn background_enabled() -> bool {
    BACKGROUND.read().map(|f| *f).unwrap_or(false) && get().enabled
}
