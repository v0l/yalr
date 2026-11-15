use std::collections::HashMap;
use std::sync::Arc;

use crate::api::public_models::{PublicModel, PublicModelDetail, PublicProvider, Snapshot};
use crate::api::public_models_math::{latency, price_of, price_of_resolved, price_ranges, uptime_percent, Price};
use crate::db::{Database, ModelPricingRow, Provider, RequestSample, VerificationRow, APPROVED};
use crate::payments::pricing::PricingResolver;
use crate::state::AppState;

const HOUR_MS: i64 = 3_600_000;
const SAMPLE_LIMIT: i64 = 50_000;

pub const POOL: &str = "pool";
pub const ROUTED: &str = "routed";

struct Facts {
    records: HashMap<i64, Provider>,
    by_name: HashMap<String, i64>,
    owners: HashMap<i64, String>,
    uptime: HashMap<i64, crate::db::HealthUptime>,
    samples: HashMap<String, HashMap<String, Vec<RequestSample>>>,
    verifications: HashMap<i64, Vec<VerificationRow>>,
}

async fn gather(db: &Database, now_ms: i64) -> Facts {
    let records: HashMap<i64, Provider> = db
        .list_providers()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.status == APPROVED)
        .map(|p| (p.id, p))
        .collect();
    let by_name = records.values().map(|p| (p.name.clone(), p.id)).collect();
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
    let mut samples: HashMap<String, HashMap<String, Vec<RequestSample>>> = HashMap::new();
    for s in db.request_samples_since(now_ms - 24 * HOUR_MS, SAMPLE_LIMIT).await.unwrap_or_default() {
        samples.entry(s.model.to_lowercase()).or_default().entry(s.provider.clone().unwrap_or_default()).or_default().push(s);
    }
    let mut verifications = HashMap::new();
    for id in records.keys() {
        verifications.insert(*id, db.latest_verifications(*id).await.unwrap_or_default());
    }
    Facts { records, by_name, owners, uptime, samples, verifications }
}

fn verification_of(provider: &Provider, model: &str, rows: &[VerificationRow]) -> &'static str {
    if provider.is_reference {
        return "reference";
    }
    let passed = rows.iter().any(|r| r.model_name.eq_ignore_ascii_case(model) && r.passed);
    if passed { "verified" } else { "unverified" }
}

fn row_for(rows: &[ModelPricingRow], provider_id: i64) -> Option<&ModelPricingRow> {
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

fn provider_view(facts: &Facts, model: &str, id: i64, price: Option<Price>) -> Option<PublicProvider> {
    let record = facts.records.get(&id)?;
    let name = record.owner_user_id.and_then(|o| facts.owners.get(&o).cloned()).unwrap_or_else(|| record.name.clone());
    let own: Vec<&RequestSample> = facts
        .samples
        .get(&model.to_lowercase())
        .and_then(|by_provider| by_provider.get(&record.name))
        .map(|all| all.iter().collect())
        .unwrap_or_default();
    let measured = latency(&own);
    let health = facts.uptime.get(&id);
    Some(PublicProvider {
        name,
        price,
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

struct Draft {
    id: String,
    kind: &'static str,
    input: Vec<String>,
    output: Vec<String>,
    context_length: Option<i32>,
    max_output_tokens: Option<i32>,
    members: Vec<(i64, Option<Price>)>,
    explicit_price: bool,
}

fn finish(facts: &Facts, draft: Draft) -> PublicModelDetail {
    let providers: Vec<PublicProvider> =
        draft.members.iter().filter_map(|(id, price)| provider_view(facts, &draft.id, *id, price.clone())).collect();
    let prices: Vec<Price> = providers.iter().filter_map(|p| p.price.clone()).collect();
    let requests_24h: i64 = facts
        .samples
        .get(&draft.id.to_lowercase())
        .map_or(0, |by_provider| by_provider.values().map(|v| v.len() as i64).sum());
    let hidden = draft.kind == POOL && requests_24h == 0 && !draft.explicit_price;
    PublicModelDetail {
        model: PublicModel {
            id: draft.id,
            kind: draft.kind,
            input_modalities: draft.input,
            output_modalities: draft.output,
            context_length: draft.context_length,
            max_output_tokens: draft.max_output_tokens,
            provider_count: providers.len(),
            price_ranges: price_ranges(&prices),
            requests_24h,
            cheapest_provider: cheapest(&providers),
            fastest_provider: fastest(&providers),
        },
        providers,
        hidden,
    }
}

fn modalities(mut list: Vec<String>) -> Vec<String> {
    list.sort();
    list.dedup();
    if list.is_empty() {
        list.push("text".into());
    }
    list
}

async fn member_price(
    resolver: Option<&Arc<PricingResolver>>,
    facts: &Facts,
    model: &str,
    id: i64,
    rows: &[ModelPricingRow],
) -> Option<Price> {
    if let (Some(resolver), Some(record)) = (resolver, facts.records.get(&id)) {
        return Some(price_of_resolved(&resolver.resolve_served(model, Some(&record.name)).await));
    }
    row_for(rows, id).map(price_of)
}

async fn pool_drafts(state: &AppState, facts: &Facts, resolver: Option<&Arc<PricingResolver>>) -> Vec<Draft> {
    let router = &state.config.router;
    if router.full_name_models().await.is_empty() {
        router.refresh_full_name_index().await;
    }
    let live = router.get_providers().await;
    let mut drafts = Vec::new();
    for name in router.full_name_models().await {
        let found = router.full_name_providers(&name).await;
        if found.is_empty() {
            continue;
        }
        let rows = state.db.list_model_pricings_by_name(&name).await.unwrap_or_default();
        let (mut input, mut output, mut limits) = (Vec::new(), Vec::new(), Vec::new());
        let mut members = Vec::new();
        for (slug, id, _) in &found {
            members.push((*id, member_price(resolver, facts, &name, *id, &rows).await));
            let Some(provider) = live.iter().find(|p| p.slug() == slug) else { continue };
            if let Some(listed) = provider.listed_modalities(&name).await {
                input.extend(listed.input_modalities);
                output.extend(listed.output_modalities);
            }
            let listed = crate::api::limits::provider_limits(state, provider, &name).await;
            if !listed.is_empty() {
                limits.push(listed);
            }
        }
        let upstream = crate::api::limits::smallest(limits);
        drafts.push(Draft {
            id: name,
            kind: POOL,
            input: modalities(input),
            output: modalities(output),
            context_length: rows.iter().filter_map(|r| r.context_window).max().or(upstream.context_length.map(|v| v as i32)),
            max_output_tokens: rows.iter().filter_map(|r| r.max_output_tokens).max().or(upstream.max_output_tokens.map(|v| v as i32)),
            members,
            explicit_price: !rows.is_empty(),
        });
    }
    drafts
}

async fn routed_drafts(state: &AppState, facts: &Facts, resolver: Option<&Arc<PricingResolver>>) -> Vec<Draft> {
    let configs = state.db.list_routing_configs().await.unwrap_or_default();
    let mut drafts = Vec::new();
    for rc in configs.iter().filter(|c| c.owner_user_id.is_none()) {
        if let Some(resolver) = resolver {
            if !resolver.resolve(&rc.name).await.is_advertised {
                continue;
            }
        }
        let mut members = Vec::new();
        for (provider, _) in state.config.router.candidate_backends(&rc.name).await {
            let Some(id) = facts.by_name.get(provider.name()).copied() else { continue };
            if members.iter().any(|(m, _)| *m == id) {
                continue;
            }
            members.push((id, member_price(resolver, facts, &rc.name, id, &[]).await));
        }
        let platform = state.db.get_model_pricing(&rc.name).await.ok().flatten();
        let upstream = crate::api::limits::routing_limits(state, &rc.name).await;
        let architecture = crate::api::modalities::routing_architecture(state, rc).await;
        let (input, output) = architecture.map_or_else(Default::default, |a| (a.input_modalities, a.output_modalities));
        drafts.push(Draft {
            id: rc.name.clone(),
            kind: ROUTED,
            input: modalities(input),
            output: modalities(output),
            context_length: platform.as_ref().and_then(|r| r.context_window).or(upstream.context_length.map(|v| v as i32)),
            max_output_tokens: platform.as_ref().and_then(|r| r.max_output_tokens).or(upstream.max_output_tokens.map(|v| v as i32)),
            members,
            explicit_price: true,
        });
    }
    drafts
}

pub async fn build(state: &AppState) -> Snapshot {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let facts = gather(&state.db, now_ms).await;
    let resolver = state.payments_state.as_ref().map(|p| p.pricing_resolver.clone());
    let mut drafts = routed_drafts(state, &facts, resolver.as_ref()).await;
    let routed: std::collections::HashSet<String> = drafts.iter().map(|d| d.id.to_lowercase()).collect();
    drafts.extend(pool_drafts(state, &facts, resolver.as_ref()).await.into_iter().filter(|d| !routed.contains(&d.id.to_lowercase())));
    let mut models: Vec<PublicModelDetail> = drafts.into_iter().map(|d| finish(&facts, d)).collect();
    models.sort_by(|a, b| a.model.id.cmp(&b.model.id));
    Snapshot { models }
}
