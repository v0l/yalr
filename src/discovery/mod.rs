pub mod event;

use std::sync::Arc;
use std::time::Duration;

use nostr::{Keys, ToBech32};
use nostr_sdk::Client;

use crate::config::DiscoveryConfig;
use crate::db::Database;
use event::{is_public_url, Listing};

const DEFAULT_RELAYS: [&str; 3] = [
    "wss://relay.damus.io",
    "wss://relay.nostr.band",
    "wss://nos.lol",
];
const REFRESH: Duration = Duration::from_secs(24 * 60 * 60);
const POLL: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(15);

pub fn spawn(config: DiscoveryConfig, db: Arc<Database>) {
    if !config.enabled {
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = run(config, db).await {
            tracing::error!(error = %e, "Nostr discovery stopped");
        }
    });
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

fn provider_id(config: &DiscoveryConfig) -> String {
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
    if let Some(onion) = config.onion_url.as_deref().map(str::trim).filter(|o| !o.is_empty()) {
        urls.push(onion.to_string());
    }
    Ok(urls)
}

async fn advertised_models(db: &Database) -> Vec<String> {
    let hidden: std::collections::HashSet<String> = db
        .list_model_pricings()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.is_advertised)
        .map(|p| p.model_name)
        .collect();
    let mut names: Vec<String> = db
        .list_routing_configs()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|rc| rc.name)
        .filter(|n| !hidden.contains(n))
        .collect();
    names.sort();
    names
}

async fn current_listing(config: &DiscoveryConfig, db: &Database, urls: Vec<String>) -> Listing {
    Listing {
        provider_id: provider_id(config),
        urls,
        models: advertised_models(db).await,
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: config.name.clone().unwrap_or_else(|| "YALR".to_string()),
        about: config
            .about
            .clone()
            .unwrap_or_else(|| "OpenAI-compatible LLM router paid over Lightning".to_string()),
    }
}

async fn run(config: DiscoveryConfig, db: Arc<Database>) -> Result<(), String> {
    let urls = urls(&config)?;
    let keys = load_keys(&config, &db).await?;
    let npub = keys.public_key().to_bech32().unwrap_or_default();

    let relays: Vec<String> = if config.relays.is_empty() {
        DEFAULT_RELAYS.iter().map(|r| r.to_string()).collect()
    } else {
        config.relays.clone()
    };
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
    loop {
        let listing = current_listing(&config, &db, urls.clone()).await;
        let due = match &published {
            Some((last, at)) => *last != listing || at.elapsed() >= REFRESH,
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
            name: None,
            about: None,
            provider_id: None,
            nsec: None,
            relays: vec![],
        }
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
        let db = Database::new("sqlite::memory:").await.unwrap();
        let first = load_keys(&config(), &db).await.unwrap();
        let second = load_keys(&config(), &db).await.unwrap();
        assert_eq!(first.public_key(), second.public_key());
    }

    #[tokio::test]
    async fn hidden_and_pool_models() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        for name in ["b-model", "a-model", "secret"] {
            db.create_routing_config(crate::db::NewRoutingConfig {
                name: name.into(),
                strategy: "round_robin".into(),
                health_check_enabled: false,
                health_check_interval_seconds: 30,
                health_check_timeout_seconds: 5,
            })
            .await
            .unwrap();
        }
        sqlx::query("INSERT INTO model_pricing (model_name, is_advertised) VALUES ('secret', 0)")
            .execute(&db.pool)
            .await
            .unwrap();
        assert_eq!(advertised_models(&db).await, vec!["a-model", "b-model"]);
    }
}
