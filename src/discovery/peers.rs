use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use nostr::{Filter, Kind};
use nostr_sdk::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::event::{is_public_url, PROVIDER_KIND};

const CACHE_TTL: Duration = Duration::from_secs(600);
const RELAY_TIMEOUT: Duration = Duration::from_secs(8);
const NODE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_BODY_BYTES: usize = 20_000_000;
const CONCURRENCY: usize = 12;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PeerPrice {
    pub node: String,
    pub model: String,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub request: f64,
}

#[derive(Clone)]
pub struct Snapshot {
    pub fetched_at: i64,
    pub nodes: usize,
    pub nodes_ok: usize,
    entries: Vec<(String, PeerPrice)>,
}

impl Snapshot {
    pub fn search(&self, query: &str, limit: usize) -> Vec<PeerPrice> {
        let wanted = model_key(query);
        let mut hits: Vec<(bool, &PeerPrice)> = self
            .entries
            .iter()
            .filter(|(key, _)| wanted.is_empty() || key.contains(&wanted))
            .map(|(key, price)| (*key == wanted, price))
            .collect();
        hits.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.input.total_cmp(&b.1.input))
                .then(a.1.node.cmp(&b.1.node))
        });
        hits.into_iter().take(limit).map(|(_, p)| p.clone()).collect()
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ModelsDoc {
    Wrapped { data: Vec<PeerModel> },
    Bare(Vec<PeerModel>),
}

#[derive(Deserialize)]
struct PeerModel {
    id: String,
    #[serde(default)]
    sats_pricing: Option<SatsPricing>,
}

#[derive(Deserialize, Default)]
struct SatsPricing {
    #[serde(default)]
    prompt: Option<f64>,
    #[serde(default)]
    completion: Option<f64>,
    #[serde(default)]
    request: Option<f64>,
    #[serde(default)]
    input_cache_read: Option<f64>,
    #[serde(default)]
    input_cache_write: Option<f64>,
}

pub fn model_key(id: &str) -> String {
    let base = id.rsplit('/').next().unwrap_or(id);
    let mut key = String::new();
    for c in base.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            key.push(c);
        } else if !key.ends_with('-') {
            key.push('-');
        }
    }
    key.trim_matches('-').to_string()
}

fn per_million(per_token: Option<f64>) -> f64 {
    per_token.filter(|v| v.is_finite()).unwrap_or(0.0) * 1_000_000.0
}

fn prices_from(node: &str, doc: ModelsDoc) -> Vec<(String, PeerPrice)> {
    let models = match doc {
        ModelsDoc::Wrapped { data } => data,
        ModelsDoc::Bare(data) => data,
    };
    models
        .into_iter()
        .filter_map(|m| {
            let p = m.sats_pricing?;
            let price = PeerPrice {
                node: node.to_string(),
                input: per_million(p.prompt),
                output: per_million(p.completion),
                cache_read: per_million(p.input_cache_read),
                cache_write: per_million(p.input_cache_write),
                request: p.request.filter(|v| v.is_finite()).unwrap_or(0.0),
                model: m.id,
            };
            (price.input > 0.0 || price.output > 0.0).then(|| (model_key(&price.model), price))
        })
        .collect()
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| {
        u.host_str().map(|h| match u.port() {
            Some(port) => format!("{h}:{port}"),
            None => h.to_string(),
        })
    })
}

fn node_urls(events: impl IntoIterator<Item = nostr::Event>, own_url: Option<&str>) -> Vec<String> {
    let own = own_url.and_then(host_of);
    let mut latest: HashMap<(String, String), nostr::Event> = HashMap::new();
    for event in events {
        let d = event
            .tags
            .iter()
            .find(|t| t.as_slice().first().map(|k| k.as_str()) == Some("d"))
            .and_then(|t| t.as_slice().get(1).cloned())
            .unwrap_or_default();
        let key = (event.pubkey.to_hex(), d);
        if latest.get(&key).is_none_or(|e| e.created_at < event.created_at) {
            latest.insert(key, event);
        }
    }
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for event in latest.values() {
        let candidate = event.tags.iter().find_map(|t| {
            let slice = t.as_slice();
            let url = slice.get(1)?;
            let usable = slice.first().map(|k| k.as_str()) == Some("u")
                && (url.starts_with("https://") || url.starts_with("http://"))
                && is_public_url(url)
                && !url.contains(".onion");
            usable.then(|| url.trim_end_matches('/').to_string())
        });
        if let Some(url) = candidate {
            let host = host_of(&url);
            if host != own && seen.insert(url.clone()) {
                urls.push(url);
            }
        }
    }
    urls.sort();
    urls
}

async fn announced_events(relays: &[String]) -> Vec<nostr::Event> {
    let client = Client::default();
    for relay in relays {
        let _ = client.add_relay(relay.as_str()).await;
    }
    client.connect().await;
    let filter = Filter::new().kind(Kind::from(PROVIDER_KIND)).limit(1000);
    let events = client.fetch_events(filter, RELAY_TIMEOUT).await;
    client.shutdown().await;
    events.map(|e| e.into_iter().collect()).unwrap_or_default()
}

async fn fetch_node(http: &reqwest::Client, base: &str) -> Option<Vec<(String, PeerPrice)>> {
    let response = http.get(format!("{base}/v1/models")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    if bytes.len() > MAX_BODY_BYTES {
        return None;
    }
    let doc: ModelsDoc = serde_json::from_slice(&bytes).ok()?;
    Some(prices_from(&host_of(base)?, doc))
}

async fn build(relays: &[String], own_url: Option<&str>) -> Snapshot {
    let urls = node_urls(announced_events(relays).await, own_url);
    let http = reqwest::Client::builder()
        .timeout(NODE_TIMEOUT)
        .user_agent("yalr-price-compare")
        .build()
        .unwrap_or_default();
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let tasks: Vec<_> = urls
        .iter()
        .cloned()
        .map(|url| {
            let http = http.clone();
            let permits = permits.clone();
            tokio::spawn(async move {
                let _permit = permits.acquire_owned().await.ok()?;
                fetch_node(&http, &url).await
            })
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.ok().flatten());
    }
    let nodes_ok = results.iter().filter(|r| r.is_some()).count();
    Snapshot {
        fetched_at: chrono::Utc::now().timestamp(),
        nodes: urls.len(),
        nodes_ok,
        entries: results.into_iter().flatten().flatten().collect(),
    }
}

static CACHE: OnceLock<Mutex<Option<(Instant, Snapshot)>>> = OnceLock::new();

pub async fn snapshot(relays: &[String], own_url: Option<&str>, refresh: bool) -> Snapshot {
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = cache.lock().await;
    if !refresh {
        if let Some((at, snap)) = guard.as_ref() {
            if at.elapsed() < CACHE_TTL {
                return snap.clone();
            }
        }
    }
    let snap = build(relays, own_url).await;
    if snap.nodes_ok > 0 {
        *guard = Some((Instant::now(), snap.clone()));
    }
    snap
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag};

    fn doc(json: &str) -> ModelsDoc {
        serde_json::from_str(json).unwrap()
    }

    fn announcement(keys: &Keys, d: &str, urls: &[&str], at: u64) -> nostr::Event {
        let mut tags = vec![Tag::parse(["d", d]).unwrap()];
        tags.extend(urls.iter().map(|u| Tag::parse(["u", u]).unwrap()));
        EventBuilder::new(Kind::from(PROVIDER_KIND), "")
            .tags(tags)
            .custom_created_at(nostr::Timestamp::from(at))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn keys_ignore_provider_prefixes_and_dot_versions() {
        assert_eq!(model_key("openrouter/anthropic/claude-opus-5.5"), "claude-opus-5-5");
        assert_eq!(model_key("claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(model_key("Claude Opus 5.5"), "claude-opus-5-5");
        assert_eq!(model_key("gemma4:31b"), "gemma4-31b");
    }

    #[test]
    fn converts_per_token_sats_to_per_million() {
        let prices = prices_from(
            "n",
            doc(r#"{"data":[{"id":"m","sats_pricing":{"prompt":0.002,"completion":0.01,"request":1.0,"input_cache_read":0.00011,"input_cache_write":null}}]}"#),
        );
        let p = &prices[0].1;
        assert!((p.input - 2000.0).abs() < 1e-6);
        assert!((p.output - 10000.0).abs() < 1e-6);
        assert!((p.cache_read - 110.0).abs() < 1e-6);
        assert_eq!(p.cache_write, 0.0);
        assert_eq!(p.request, 1.0);
    }

    #[test]
    fn skips_models_without_sats_prices_and_accepts_bare_lists() {
        let prices = prices_from(
            "n",
            doc(r#"[{"id":"a"},{"id":"b","sats_pricing":{"prompt":0.0,"completion":0.0}},{"id":"c","sats_pricing":{"prompt":0.001}}]"#),
        );
        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].1.model, "c");
    }

    #[test]
    fn search_ranks_exact_matches_first_then_cheapest() {
        let entry = |model: &str, node: &str, input: f64| {
            let price = PeerPrice { node: node.into(), model: model.into(), input, output: 0.0, cache_read: 0.0, cache_write: 0.0, request: 0.0 };
            (model_key(model), price)
        };
        let snap = Snapshot {
            fetched_at: 0,
            nodes: 3,
            nodes_ok: 3,
            entries: vec![
                entry("claude-sonnet-5.5-fast", "a", 1.0),
                entry("anthropic/claude-sonnet-5.5", "b", 3000.0),
                entry("claude-sonnet-5-5", "c", 2500.0),
                entry("gpt-5", "d", 1.0),
            ],
        };
        let hits: Vec<_> = snap.search("claude-sonnet-5.5", 10).into_iter().map(|p| p.node).collect();
        assert_eq!(hits, vec!["c", "b", "a"]);
        assert_eq!(snap.search("claude-sonnet-5.5", 1).len(), 1);
        assert_eq!(snap.search("", 10).len(), 4);
    }

    #[test]
    fn node_urls_keep_the_latest_announcement_and_drop_own_and_onion_and_private() {
        let a = Keys::generate();
        let b = Keys::generate();
        let events = vec![
            announcement(&a, "a", &["https://old.example"], 10),
            announcement(&a, "a", &["https://new.example/"], 20),
            announcement(&b, "me", &["https://yalr.example"], 20),
            announcement(&Keys::generate(), "o", &["http://abc.onion", "http://localhost:3000"], 20),
        ];
        assert_eq!(node_urls(events, Some("https://yalr.example")), vec!["https://new.example"]);
    }
}
