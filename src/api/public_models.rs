use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use tokio::sync::Mutex;

use crate::api::public_models_math::{latency, price_of, price_ranges, uptime_percent, ModelPriceRange, Price};
use crate::db::{Database, ModelPricingRow, Provider, RequestSample, VerificationRow};
use crate::state::AppState;

const CACHE_TTL: Duration = Duration::from_secs(60);
const HOUR_MS: i64 = 3_600_000;
const SAMPLE_LIMIT: i64 = 10_000;

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
}

#[derive(Serialize)]
pub struct PublicModelList {
    pub models: Vec<PublicModel>,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    pub models: Vec<PublicModelDetail>,
}

#[derive(Clone)]
pub struct PublicCache {
    ttl: Duration,
    inner: Arc<Mutex<Option<(Instant, Arc<Snapshot>)>>>,
}

impl Default for PublicCache {
    fn default() -> Self {
        Self::with_ttl(CACHE_TTL)
    }
}

impl PublicCache {
    pub fn with_ttl(ttl: Duration) -> Self {
        Self { ttl, inner: Default::default() }
    }

    pub async fn snapshot(&self, state: &AppState) -> Arc<Snapshot> {
        let mut slot = self.inner.lock().await;
        if let Some((at, snapshot)) = slot.as_ref() {
            if at.elapsed() < self.ttl {
                return snapshot.clone();
            }
        }
        let snapshot = Arc::new(build(state).await);
        *slot = Some((Instant::now(), snapshot.clone()));
        snapshot
    }
}

fn verification_of(provider: &Provider, model: &str, rows: &[VerificationRow]) -> &'static str {
    if provider.is_reference {
        return "reference";
    }
    let passed = rows.iter().any(|r| r.model_name.eq_ignore_ascii_case(model) && r.passed);
    if passed { "verified" } else { "unverified" }
}

fn row_for<'a>(rows: &'a [ModelPricingRow], provider_id: i64) -> Option<&'a ModelPricingRow> {
    rows.iter()
        .find(|r| r.provider_id == Some(provider_id))
        .or_else(|| rows.iter().find(|r| r.provider_id.is_none()))
}

fn cheapest(providers: &[PublicProvider]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in providers.iter().filter_map(|p| p.price.as_ref()) {
        *counts.entry(p.currency.as_str()).or_default() += 1;
    }
    let currency = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))?.0.to_string();
    providers
        .iter()
        .filter_map(|p| p.price.as_ref().filter(|pr| pr.currency == currency).map(|pr| (p, pr)))
        .min_by(|a, b| (a.1.input_per_1m + a.1.output_per_1m).total_cmp(&(b.1.input_per_1m + b.1.output_per_1m)))
        .map(|(p, _)| p.name.clone())
}

fn fastest(providers: &[PublicProvider]) -> Option<String> {
    providers
        .iter()
        .filter_map(|p| p.tokens_per_second_p50.map(|t| (p, t)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(p, _)| p.name.clone())
}

struct Facts {
    records: HashMap<i64, Provider>,
    owners: HashMap<i64, String>,
    uptime: HashMap<i64, crate::db::HealthUptime>,
    samples: HashMap<String, Vec<RequestSample>>,
    verifications: HashMap<i64, Vec<VerificationRow>>,
}

async fn gather(db: &Database, provider_ids: &[i64], now_ms: i64) -> Facts {
    let records: HashMap<i64, Provider> = db
        .list_providers()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| provider_ids.contains(&p.id))
        .map(|p| (p.id, p))
        .collect();
    let mut owners = HashMap::new();
    for owner in records.values().filter_map(|p| p.owner_user_id) {
        if let Ok(Some(user)) = db.get_user_by_id(owner).await {
            if let Some(name) = user.username {
                owners.insert(owner, name);
            }
        }
    }
    let uptime = db
        .health_uptime(now_ms - 24 * HOUR_MS, now_ms - 7 * 24 * HOUR_MS)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|u| (u.provider_id, u))
        .collect();
    let mut samples = HashMap::new();
    let mut verifications = HashMap::new();
    for provider in records.values() {
        let found = db.provider_request_samples(&provider.name, now_ms - 24 * HOUR_MS, SAMPLE_LIMIT).await;
        samples.insert(provider.name.clone(), found.unwrap_or_default());
        verifications.insert(provider.id, db.latest_verifications(provider.id).await.unwrap_or_default());
    }
    Facts { records, owners, uptime, samples, verifications }
}

fn provider_view(facts: &Facts, model: &str, id: i64, row: Option<&ModelPricingRow>) -> Option<PublicProvider> {
    let record = facts.records.get(&id)?;
    let name = record
        .owner_user_id
        .and_then(|o| facts.owners.get(&o).cloned())
        .unwrap_or_else(|| record.name.clone());
    let own: Vec<&RequestSample> = facts
        .samples
        .get(&record.name)
        .map(|all| all.iter().filter(|s| s.model.eq_ignore_ascii_case(model)).collect())
        .unwrap_or_default();
    let measured = latency(&own);
    let health = facts.uptime.get(&id);
    Some(PublicProvider {
        name,
        price: row.map(price_of),
        verification: verification_of(record, model, facts.verifications.get(&id).map_or(&[], Vec::as_slice)),
        uptime_24h: health.and_then(|h| uptime_percent(h.checks_24h, h.healthy_24h)),
        uptime_7d: health.and_then(|h| uptime_percent(h.checks_7d, h.healthy_7d)),
        requests_24h: measured.requests,
        error_rate: measured.error_rate,
        ttft_p50_ms: measured.ttft_p50_ms,
        ttft_p95_ms: measured.ttft_p95_ms,
        tokens_per_second_p50: measured.tokens_per_second_p50,
        last_healthy_at_ms: health.and_then(|h| h.last_healthy_ms),
        listed_since: record.created_at,
    })
}

async fn build(state: &AppState) -> Snapshot {
    let router = &state.config.router;
    if router.full_name_models().await.is_empty() {
        router.refresh_full_name_index().await;
    }
    let names = router.full_name_models().await;
    let mut members = Vec::new();
    for name in names {
        let found = router.full_name_providers(&name).await;
        if !found.is_empty() {
            members.push((name, found));
        }
    }
    let mut ids: Vec<i64> = members.iter().flat_map(|(_, m)| m.iter().map(|(_, id, _)| *id)).collect();
    ids.sort_unstable();
    ids.dedup();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let facts = gather(&state.db, &ids, now_ms).await;
    let live = router.get_providers().await;

    let mut models = Vec::new();
    for (name, found) in members {
        let rows = state.db.list_model_pricings_by_name(&name).await.unwrap_or_default();
        let providers: Vec<PublicProvider> = found
            .iter()
            .filter_map(|(_, id, _)| provider_view(&facts, &name, *id, row_for(&rows, *id)))
            .collect();
        let mut input = Vec::new();
        let mut output = Vec::new();
        for (slug, _, _) in &found {
            let Some(provider) = live.iter().find(|p| p.slug() == slug) else { continue };
            if let Some(listed) = provider.listed_modalities(&name).await {
                input.extend(listed.input_modalities);
                output.extend(listed.output_modalities);
            }
        }
        for list in [&mut input, &mut output] {
            list.sort();
            list.dedup();
        }
        if input.is_empty() {
            input.push("text".into());
        }
        if output.is_empty() {
            output.push("text".into());
        }
        let used: Vec<&ModelPricingRow> = found.iter().filter_map(|(_, id, _)| row_for(&rows, *id)).collect();
        let prices: Vec<Price> = used.iter().map(|r| price_of(r)).collect();
        models.push(PublicModelDetail {
            model: PublicModel {
                id: name,
                input_modalities: input,
                output_modalities: output,
                context_length: used.iter().filter_map(|r| r.context_window).max(),
                max_output_tokens: used.iter().filter_map(|r| r.max_output_tokens).max(),
                provider_count: providers.len(),
                price_ranges: price_ranges(&prices),
                requests_24h: providers.iter().map(|p| p.requests_24h).sum(),
                cheapest_provider: cheapest(&providers),
                fastest_provider: fastest(&providers),
            },
            providers,
        });
    }
    models.sort_by(|a, b| a.model.id.cmp(&b.model.id));
    Snapshot { models }
}

fn cached<T: Serialize>(body: T) -> Response {
    ([(header::CACHE_CONTROL, "public, max-age=60")], Json(body)).into_response()
}

pub async fn list(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = state.public_cache.snapshot(&state).await;
    cached(PublicModelList { models: snapshot.models.iter().map(|m| m.model.clone()).collect() })
}

pub async fn detail(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    let snapshot = state.public_cache.snapshot(&state).await;
    match snapshot.models.iter().find(|m| m.model.id.eq_ignore_ascii_case(&name)) {
        Some(found) => cached(found),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
