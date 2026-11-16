use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::api::public_models_build::build;
use crate::api::public_models_math::{ModelPriceRange, Price};
use crate::state::AppState;

const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Serialize, Debug, Clone)]
pub struct PublicProvider {
    pub name: String,
    pub price: Option<Price>,
    pub verification: &'static str,
    pub uptime_24h: Option<f64>,
    pub uptime_7d: Option<f64>,
    pub requests_24h: i64,
    pub error_rate: Option<f64>,
    pub ttft_p50_ms: Option<f64>,
    pub ttft_p95_ms: Option<f64>,
    pub tokens_per_second_p50: Option<f64>,
    pub last_healthy_at_ms: Option<i64>,
    pub listed_since: crate::db::Timestamp,
}

#[derive(Serialize, Debug, Clone)]
pub struct PublicModel {
    pub id: String,
    pub kind: &'static str,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub context_length: Option<i32>,
    pub max_output_tokens: Option<i32>,
    pub provider_count: usize,
    pub price_ranges: Vec<ModelPriceRange>,
    pub requests_24h: i64,
    pub cheapest_provider: Option<String>,
    pub fastest_provider: Option<String>,
}

#[derive(Serialize, Debug, Clone)]
pub struct PublicModelDetail {
    #[serde(flatten)]
    pub model: PublicModel,
    pub providers: Vec<PublicProvider>,
    #[serde(skip)]
    pub hidden: bool,
}

#[derive(Serialize)]
pub struct PublicModelList {
    pub models: Vec<PublicModel>,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub models: Vec<PublicModelDetail>,
}

#[derive(Default)]
struct Slot {
    current: Option<(Instant, Arc<Snapshot>)>,
    refreshing: bool,
}

#[derive(Clone)]
pub struct PublicCache {
    ttl: Duration,
    slot: Arc<std::sync::Mutex<Slot>>,
    first_build: Arc<Mutex<()>>,
}

impl Default for PublicCache {
    fn default() -> Self {
        Self::with_ttl(CACHE_TTL)
    }
}

impl PublicCache {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl, slot: Default::default(), first_build: Default::default() }
    }

    fn store(&self, snapshot: Arc<Snapshot>) {
        let mut slot = self.slot.lock().unwrap();
        slot.current = Some((Instant::now(), snapshot));
        slot.refreshing = false;
    }

    fn lookup(&self) -> Option<(Instant, Arc<Snapshot>)> {
        self.slot.lock().unwrap().current.clone()
    }

    fn claim_refresh(&self) -> bool {
        let mut slot = self.slot.lock().unwrap();
        !std::mem::replace(&mut slot.refreshing, true)
    }

    pub async fn snapshot(&self, state: &Arc<AppState>) -> Arc<Snapshot> {
        if let Some((at, snapshot)) = self.lookup() {
            if at.elapsed() >= self.ttl && self.claim_refresh() {
                let (cache, state) = (self.clone(), state.clone());
                tokio::spawn(async move { cache.store(Arc::new(build(&state).await)) });
            }
            return snapshot;
        }
        let _gate = self.first_build.lock().await;
        if let Some((_, snapshot)) = self.lookup() {
            return snapshot;
        }
        let snapshot = Arc::new(build(state).await);
        self.store(snapshot.clone());
        snapshot
    }
}

fn cached<T: Serialize>(body: T) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=60")], Json(body)).into_response()
}

#[derive(serde::Deserialize, Default)]
pub struct ListQuery {
    #[serde(default)]
    pub all: bool,
}

pub async fn list(State(state): State<Arc<AppState>>, Query(query): Query<ListQuery>) -> Response {
    let snapshot = state.public_cache.snapshot(&state).await;
    let models = snapshot.models.iter().filter(|m| query.all || !m.hidden).map(|m| m.model.clone()).collect();
    cached(PublicModelList { models })
}

pub async fn detail(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let snapshot = state.public_cache.snapshot(&state).await;
    match snapshot.models.iter().find(|m| m.model.id.eq_ignore_ascii_case(&name)) {
        Some(found) => cached(found),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
