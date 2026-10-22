use axum::{extract::{Query, State}, Json};
use serde::{Deserialize, Serialize};

use crate::discovery::peers::{self, PeerPrice};
use crate::state::AppState;

const RESULT_LIMIT: usize = 150;

#[derive(Deserialize)]
pub struct MarketQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    refresh: bool,
}

#[derive(Serialize)]
pub struct MarketResponse {
    fetched_at: i64,
    nodes: usize,
    nodes_ok: usize,
    results: Vec<PeerPrice>,
}

pub async fn market(
    State(state): State<std::sync::Arc<AppState>>,
    Query(query): Query<MarketQuery>,
) -> Json<MarketResponse> {
    let discovery = state.config.discovery_config.as_ref();
    let relays = crate::discovery::relays(discovery);
    let own_url = discovery.map(|d| d.http_url.as_str());
    let snapshot = peers::snapshot(&relays, own_url, query.refresh).await;
    Json(MarketResponse {
        fetched_at: snapshot.fetched_at,
        nodes: snapshot.nodes,
        nodes_ok: snapshot.nodes_ok,
        results: snapshot.search(&query.q, RESULT_LIMIT),
    })
}

#[derive(Serialize)]
pub struct ListingStatus {
    discovery_enabled: bool,
    provider_id: Option<String>,
    npub: Option<String>,
    relays: Vec<String>,
    http_url: Option<String>,
}

pub async fn listing(State(state): State<std::sync::Arc<AppState>>) -> Json<ListingStatus> {
    let discovery = state.config.discovery_config.as_ref().filter(|d| d.enabled);
    let npub = match discovery {
        Some(d) => crate::discovery::npub(d, &state.db).await,
        None => None,
    };
    Json(ListingStatus {
        discovery_enabled: discovery.is_some(),
        provider_id: discovery.map(crate::discovery::provider_id),
        npub,
        relays: discovery.map(|d| crate::discovery::relays(Some(d))).unwrap_or_default(),
        http_url: discovery.map(|d| d.http_url.trim_end_matches('/').to_string()),
    })
}
