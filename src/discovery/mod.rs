pub mod event;
pub mod peers;
pub mod profile;

use std::sync::Arc;
use std::time::Duration;

use nostr::{Keys, ToBech32};
use nostr_sdk::Client;

use crate::config::DiscoveryConfig;
use crate::db::Database;
use crate::state::AppState;
use event::{is_public_url, Listing, ModelDetail, SatsPricing};

pub(crate) const DEFAULT_RELAYS: [&str; 5] = [
    "wss://relay.damus.io",
    "wss://purplepag.es",
    "wss://nos.lol",
    "wss://relay.routstr.com",
    "wss://relay.primal.net",
];
const DEFAULT_ABOUT: &str = "OpenAI-compatible LLM router with uncensored and open-weight models, paid per request with Lightning or card. No account needed.";
const REFRESH: Duration = Duration::from_secs(24 * 60 * 60);
const POLL: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(15);
const PARTIAL_RETRY: Duration = Duration::from_secs(300);
const PARTIAL_RETRY_MAX: Duration = Duration::from_secs(60 * 60);

fn partial_retry_delay(attempt: u32) -> Duration {
    PARTIAL_RETRY
        .checked_mul(1u32 << attempt.min(16))
        .map_or(PARTIAL_RETRY_MAX, |d| d.min(PARTIAL_RETRY_MAX))
}

pub fn spawn(config: DiscoveryConfig, state: Arc<AppState>) {
    if !config.enabled {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = run(config, state).await {
            tracing::error!(error = %e, "Nostr discovery stopped");
        }
    });
}

pub fn relays(config: Option<&DiscoveryConfig>) -> Vec<String> {
    match config {
        Some(c) if !c.relays.is_empty() => c.relays.clone(),
        _ => DEFAULT_RELAYS.iter().map(|r| r.to_string()).collect(),
    }
}

pub async fn npub(config: &DiscoveryConfig, db: &Database) -> Option<String> {
    load_keys(config, db).await.ok()?.public_key().to_bech32().ok()
}

async fn load_keys(config: &DiscoveryConfig, db: &Database) -> Result<Keys, String> {
    if let Some(nsec) = config.nsec.as_deref().filter(|n| !n.trim().is_empty()) {
        return Keys::parse(nsec.trim()).map_err(|e| format!("invalid discovery nsec: {e}"));
    }
    if db.nostr_secret_key().await.map_err(|e| e.to_string())?.is_none() {
        let generated = Keys::generate();
        let hex = generated.secret_key().to_secret_hex();
        db.insert_nostr_secret_key(&hex).await.map_err(|e| e.to_string())?;
    }
    let stored = db
        .nostr_secret_key()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("nostr identity missing after insert")?;
    Keys::parse(&stored).map_err(|e| e.to_string())
}

pub fn provider_id(config: &DiscoveryConfig) -> String {
    config
        .provider_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .or_else(|| {
            url::Url::parse(&config.http_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
        })
        .unwrap_or_else(|| "yalr".to_string())
}

fn urls(config: &DiscoveryConfig) -> Result<Vec<String>, String> {
    if !is_public_url(&config.http_url) {
        return Err(format!("discovery http_url {} is not publicly routable", config.http_url));
    }
    let mut urls = vec![config.http_url.trim_end_matches('/').to_string()];
    for extra in config.extra_urls.iter().map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty()) {
        if !is_public_url(extra) {
            return Err(format!("discovery extra url {extra} is not publicly routable"));
        }
        if !urls.iter().any(|u| u == extra) {
            urls.push(extra.to_string());
        }
    }
    if let Some(onion) = config.onion_url.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        urls.push(onion.to_string());
    }
    Ok(urls)
}

async fn advertised_models(state: &Arc<AppState>) -> Vec<ModelDetail> {
    crate::api::models::public_model_list(state)
        .await
        .into_iter()
        .map(|m| ModelDetail {
            vision: m.architecture.input_modalities.iter().any(|i| i == "image"),
            id: m.id,
            context_length: m.context_length,
            max_output_tokens: m.top_provider.max_completion_tokens,
            pricing: SatsPricing {
                prompt: m.sats_pricing.prompt,
                completion: m.sats_pricing.completion,
                request: m.sats_pricing.request,
                input_cache_read: m.sats_pricing.input_cache_read,
                input_cache_write: m.sats_pricing.input_cache_write,
            },
        })
        .collect()
}

async fn current_listing(config: &DiscoveryConfig, state: &Arc<AppState>, urls: Vec<String>) -> Listing {
    Listing {
        provider_id: provider_id(config),
        urls,
        models: advertised_models(state).await,
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: config.name.clone().unwrap_or_else(|| "YALR".to_string()),
        about: config.about.clone().unwrap_or_else(|| DEFAULT_ABOUT.to_string()),
    }
}

async fn run(config: DiscoveryConfig, state: Arc<AppState>) -> Result<(), String> {
    let db = state.db.clone();
    let urls = urls(&config)?;
    let keys = load_keys(&config, &db).await?;
    let npub = keys.public_key().to_bech32().unwrap_or_default();

    let relays = relays(Some(&config));
    let client = Client::new(keys.clone());
    for relay in &relays {
        if let Err(e) = client.add_relay(relay.as_str()).await {
            tracing::warn!(relay = %relay, error = %e, "Invalid discovery relay");
        }
    }
    client.connect().await;
    client.wait_for_connection(Duration::from_secs(10)).await;
    tracing::info!(npub = %npub, "Nostr discovery enabled");

    let mut published: Option<(Listing, tokio::time::Instant)> = None;
    let mut partial_attempts: Option<u32> = None;
    loop {
        let listing = current_listing(&config, &state, urls.clone()).await;
        let due = match &published {
            Some((last, at)) => {
                *last != listing
                    || at.elapsed() >= REFRESH
                    || partial_attempts.is_some_and(|n| at.elapsed() >= partial_retry_delay(n))
            }
            None => true,
        };
        if due {
            let event = listing.sign(&keys).map_err(|e| e.to_string())?;
            match client.send_event(&event).await {
                Ok(output) if !output.success.is_empty() => {
                    tracing::info!(
                        relays = output.success.len(),
                        models = listing.models.len(),
                        "Published Routstr provider announcement"
                    );
                    let changed = published.as_ref().is_none_or(|(last, _)| *last != listing);
                    partial_attempts = if output.success.len() < relays.len() {
                        Some(if changed { 0 } else { partial_attempts.map_or(0, |n| n + 1) })
                    } else {
                        None
                    };
                    published = Some((listing, tokio::time::Instant::now()));
                }
                Ok(output) => {
                    tracing::warn!(failed = output.failed.len(), "No relay accepted the provider announcement");
                }
                Err(e) => tracing::warn!(error = %e, "Failed to publish provider announcement"),
            }
        }
        tokio::time::sleep(if published.is_some() { POLL } else { RETRY }).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> DiscoveryConfig {
        DiscoveryConfig {
            enabled: true,
            http_url: "https://yalr.v0l.io/".into(),
            onion_url: None,
            extra_urls: vec![],
            name: None,
            about: None,
            provider_id: None,
            nsec: None,
            relays: vec![],
        }
    }

    #[test]
    fn partial_retry_backs_off_to_an_hour() {
        let minutes: Vec<u64> = (0..7).map(|n| partial_retry_delay(n).as_secs() / 60).collect();
        assert_eq!(minutes, vec![5, 10, 20, 40, 60, 60, 60]);
        assert_eq!(partial_retry_delay(u32::MAX), PARTIAL_RETRY_MAX);
    }

    #[test]
    fn provider_id_defaults_to_host() {
        assert_eq!(provider_id(&config()), "yalr.v0l.io");
    }

    #[test]
    fn urls_trim_trailing_slash_and_add_onion() {
        let mut c = config();
        c.onion_url = Some("http://abc.onion".into());
        assert_eq!(urls(&c).unwrap(), vec!["https://yalr.v0l.io", "http://abc.onion"]);
    }

    #[test]
    fn private_url_is_refused() {
        let mut c = config();
        c.http_url = "http://localhost:3000".into();
        assert!(urls(&c).is_err());
    }

    #[tokio::test]
    async fn generated_identity_is_stable() {
        let db = Database::test().await;
        let first = load_keys(&config(), &db).await.unwrap();
        let second = load_keys(&config(), &db).await.unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }

    #[test]
    fn extra_urls_are_published_once_and_must_be_public() {
        let mut c = config();
        c.extra_urls = vec!["https://app.yalr.net/".into(), "https://yalr.v0l.io".into()];
        assert_eq!(urls(&c).unwrap(), vec!["https://yalr.v0l.io", "https://app.yalr.net"]);
        c.extra_urls = vec!["http://10.0.0.1".into()];
        assert!(urls(&c).is_err());
    }

    #[test]
    fn default_relays_include_routstr_and_primal() {
        let defaults = relays(None);
        assert!(defaults.contains(&"wss://relay.routstr.com".to_string()));
        assert!(defaults.contains(&"wss://relay.primal.net".to_string()));
    }

    #[tokio::test]
    async fn the_listing_uses_the_public_models_data() {
        let state = crate::api::public_feed_tests::production_state().await;
        let listing = current_listing(&config(), &state, vec!["https://yalr.v0l.io".into()]).await;
        let public = crate::api::models::public_model_list(&state).await;
        assert_eq!(listing.models.len(), public.len());
        assert_eq!(listing.models[0].id, public[0].id);
        assert_eq!(listing.models[0].context_length, 1_000_000);
        assert_eq!(listing.models[0].pricing.prompt, public[0].sats_pricing.prompt);
        assert!(listing.models[0].pricing.prompt > 0.0);
        assert_eq!(listing.version, env!("CARGO_PKG_VERSION"));
        assert!(listing.about.contains("Lightning") && listing.about.contains("uncensored"));
    }
}
