use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::tenant_tests::tenancy;
use crate::db::{FiatPrices, NewModelPricing, NewProvider, Record};
use crate::providers::{ChatRequest, Provider, ProviderError, StreamingChunk};
use crate::ChatCompletionResponse;

const MODEL: &str = "qwen/qwen3.8-27b";
const HOUR_MS: i64 = 3_600_000;

struct Stub(String);

#[async_trait]
impl Provider for Stub {
    fn name(&self) -> &str {
        &self.0
    }
    fn slug(&self) -> &str {
        &self.0
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![crate::Model { id: MODEL.into(), object: "model".into(), created: 0, owned_by: "x".into() }])
    }
    async fn chat_completions(&self, _request: &ChatRequest) -> Result<ChatCompletionResponse, ProviderError> {
        Err(ProviderError::Unsupported("stub".into()))
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamingChunk, ProviderError>>, ProviderError> {
        Err(ProviderError::Unsupported("stub".into()))
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

struct Seed {
    owner: &'static str,
    name: &'static str,
    status: &'static str,
    currency: &'static str,
    input: i64,
    output: i64,
}

async fn add(t: &super::tenant_tests::Tenancy, seed: Seed) -> i64 {
    let owner = t.state.db.add_test_user(seed.owner, false).await;
    add_for(t, &owner, seed).await
}

async fn add_for(t: &super::tenant_tests::Tenancy, owner: &crate::db::User, seed: Seed) -> i64 {
    let db = &t.state.db;
    let new = NewProvider {
        name: seed.name,
        slug: seed.name,
        base_url: "https://private-host.example.com/v1",
        api_key: Some("sk-private-key"),
        provider_type: None,
    };
    let created = db.create_owned_provider(owner.id, new).await.unwrap();
    let record = db.set_provider_status(created.id, seed.status, None, false).await.unwrap();
    let sats = seed.currency == "sats";
    db.create_model_pricing(NewModelPricing {
        model_name: MODEL,
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: sats.then_some(seed.input),
        price_per_1m_output_sats: sats.then_some(seed.output),
        price_per_1m_cache_read_sats: None,
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: None,
        context_window: Some(32_768),
        max_output_tokens: Some(4_096),
        owner_user_id: Some(owner.id),
        provider_id: Some(record.id),
        currency: seed.currency,
        fiat: if sats { FiatPrices::default() } else { FiatPrices { input: Some(seed.input), output: Some(seed.output), ..Default::default() } },
    })
    .await
    .unwrap();
    t.state.config.router.add_provider(Arc::new(Stub(seed.name.to_string()))).await;
    record.id
}

fn request(id: &str, provider: &str, status: &str, ttft: i64, total: i64, tokens: i64) -> Record {
    Record {
        request_id: id.into(),
        started_at_ms: chrono::Utc::now().timestamp_millis() - HOUR_MS,
        endpoint: "/v1/chat/completions".into(),
        model: MODEL.into(),
        provider: Some(provider.into()),
        status: status.into(),
        attempts: 1,
        first_event_ms: Some(ttft),
        total_ms: total,
        completion_tokens: Some(tokens),
        ..Default::default()
    }
}

async fn market() -> (super::tenant_tests::Tenancy, i64) {
    let t = tenancy().await;
    let fast = add(&t, Seed { owner: "fastco", name: "fast-p", status: "approved", currency: "usd", input: 100_000, output: 300_000 }).await;
    add(&t, Seed { owner: "satco", name: "sat-p", status: "approved", currency: "sats", input: 500, output: 900 }).await;
    add(&t, Seed { owner: "newco", name: "pending-p", status: "pending", currency: "usd", input: 1, output: 1 }).await;
    add(&t, Seed { owner: "badco", name: "rejected-p", status: "suspended", currency: "usd", input: 1, output: 1 }).await;
    let now = chrono::Utc::now().timestamp_millis();
    for (i, healthy) in [true, true, true, false].into_iter().enumerate() {
        t.state.db.record_health_check(fast, healthy, now - (i as i64 + 1) * HOUR_MS).await.unwrap();
    }
    t.state.db.record_health_check(fast, true, now - 3 * 24 * HOUR_MS).await.unwrap();
    t.state
        .db
        .insert_request_logs(&[
            request("r1", "fast-p", "ok", 100, 1_100, 100),
            request("r2", "fast-p", "ok", 200, 2_200, 400),
            request("r3", "fast-p", "error", 5_000, 5_000, 0),
            request("r4", "sat-p", "ok", 900, 1_900, 50),
        ])
        .await
        .unwrap();
    t.state.config.router.refresh_full_name_index().await;
    (t, fast)
}

fn provider<'a>(body: &'a Value, owner: &str) -> &'a Value {
    body["providers"].as_array().unwrap().iter().find(|p| p["owner"] == owner).unwrap_or_else(|| panic!("{owner} missing in {body}"))
}

#[tokio::test]
async fn public_models_need_no_login_and_list_only_approved_providers() {
    let (t, _) = market().await;
    let (status, list) = t.get("", "/api/public/models").await;
    assert_eq!(status, 200, "{list}");
    let models = list["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], MODEL);
    assert_eq!(models[0]["provider_count"], 2);
    assert_eq!(models[0]["context_length"], 32_768);

    let (status, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    assert_eq!(status, 200, "{detail}");
    let names: Vec<&str> = detail["providers"].as_array().unwrap().iter().map(|p| p["owner"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"fastco") && names.contains(&"satco"), "{names:?}");
    assert_eq!(t.get("", "/api/public/models/qwen/nothing").await.0, 404);
}

#[tokio::test]
async fn public_models_never_expose_private_fields() {
    let (t, _) = market().await;
    let (_, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    let (_, list) = t.get("", "/api/public/models").await;
    for body in [detail.to_string(), list.to_string()] {
        for forbidden in ["private-host", "example.com", "base_url", "sk-private", "api_key", "pending-p", "rejected-p", "newco", "badco", "owner_user_id", "email"] {
            assert!(!body.contains(forbidden), "{forbidden} leaked in {body}");
        }
    }
}

#[tokio::test]
async fn uptime_and_latency_come_from_seeded_rows() {
    let (t, _) = market().await;
    let (_, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    let fast = provider(&detail, "fastco");
    assert_eq!(fast["uptime_24h"], 75.0);
    assert_eq!(fast["uptime_7d"], 80.0);
    assert_eq!(fast["requests_24h"], 3);
    assert_eq!(fast["error_rate"], 33.33);
    assert_eq!(fast["ttft_p50_ms"], 100.0);
    assert_eq!(fast["ttft_p95_ms"], 200.0);
    assert_eq!(fast["tokens_per_second_p50"], 100.0);
    assert!(fast["last_healthy_at_ms"].as_i64().is_some());
    assert_eq!(fast["verification"], "unverified");

    let sat = provider(&detail, "satco");
    assert_eq!(sat["uptime_24h"], Value::Null);
    assert_eq!(sat["requests_24h"], 1);
    assert_eq!(detail["requests_24h"], 4);
    assert_eq!(detail["fastest_provider"], "fast-p");
}

#[tokio::test]
async fn price_ranges_keep_currencies_apart_as_each_provider_set_them() {
    let (t, _) = market().await;
    let (_, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    let ranges = detail["price_ranges"].as_array().unwrap();
    assert_eq!(ranges.len(), 2);
    let usd = ranges.iter().find(|r| r["currency"] == "usd").unwrap();
    assert_eq!((usd["min_input"].as_f64(), usd["max_output"].as_f64()), (Some(1.0), Some(3.0)));
    let sats = ranges.iter().find(|r| r["currency"] == "sats").unwrap();
    assert_eq!((sats["min_input"].as_f64(), sats["max_output"].as_f64()), (Some(500.0), Some(900.0)));
    assert_eq!(provider(&detail, "satco")["price"]["currency"], "sats");
}

#[tokio::test]
async fn a_verified_provider_shows_verified() {
    let (t, fast) = market().await;
    t.state
        .db
        .insert_verification(crate::db::NewVerification {
            run_id: "run",
            provider_id: fast,
            model_name: MODEL,
            reference_provider_id: None,
            outcome: "passed",
            passed: true,
            reason: None,
            scores: "{}",
            results: "[]",
        })
        .await
        .unwrap();
    let (_, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    assert_eq!(provider(&detail, "fastco")["verification"], "verified");
    assert_eq!(provider(&detail, "satco")["verification"], "unverified");
}

#[tokio::test]
async fn responses_are_cached_for_the_ttl() {
    let (t, _) = market().await;
    let (_, first) = t.get("", "/api/public/models").await;
    assert_eq!(first["models"][0]["provider_count"], 2);

    add(&t, Seed { owner: "lateco", name: "late-p", status: "approved", currency: "usd", input: 1, output: 1 }).await;
    t.state.config.router.refresh_full_name_index().await;
    let (_, cached) = t.get("", "/api/public/models").await;
    assert_eq!(cached["models"][0]["provider_count"], 2);

    let fresh = crate::api::public_models::PublicCache::with_ttl(std::time::Duration::ZERO);
    let snapshot = fresh.snapshot(&t.state).await;
    assert_eq!(snapshot.models[0].model.provider_count, 3);
}

#[tokio::test]
async fn requests_routed_through_a_config_count_for_the_upstream_model() {
    let (t, _) = market().await;
    t.state
        .db
        .insert_request_logs(&[Record {
            model: "my-config".into(),
            upstream_model: Some(MODEL.into()),
            route: Some("my-config > fast-p".into()),
            ..request("r5", "fast-p", "ok", 100, 1_100, 100)
        }])
        .await
        .unwrap();
    let (_, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    assert_eq!(detail["requests_24h"], 5);
    assert_eq!(provider(&detail, "fastco")["requests_24h"], 4);
}

#[tokio::test]
async fn one_owner_with_two_providers_gets_distinct_cheapest_and_fastest_keys() {
    let t = tenancy().await;
    let owner = t.state.db.add_test_user("multico", false).await;
    add_for(&t, &owner, Seed { owner: "multico", name: "ripper", status: "approved", currency: "sats", input: 800, output: 900 }).await;
    add_for(&t, &owner, Seed { owner: "multico", name: "spark", status: "approved", currency: "sats", input: 500, output: 600 }).await;
    t.state
        .db
        .insert_request_logs(&[request("m1", "ripper", "ok", 100, 1_100, 1_000), request("m2", "spark", "ok", 100, 1_100, 100)])
        .await
        .unwrap();
    t.state.config.router.refresh_full_name_index().await;
    let (status, detail) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    assert_eq!(status, 200, "{detail}");
    assert_eq!(detail["cheapest_provider"], "spark");
    assert_eq!(detail["fastest_provider"], "ripper");
    let rows = detail["providers"].as_array().unwrap();
    let mut keys: Vec<(&str, &str, &str)> =
        rows.iter().map(|p| (p["key"].as_str().unwrap(), p["provider"].as_str().unwrap(), p["owner"].as_str().unwrap())).collect();
    keys.sort();
    assert_eq!(keys, vec![("ripper", "ripper", "multico"), ("spark", "spark", "multico")]);
    assert!(rows.iter().all(|p| p.get("id").is_none()));
    for body in [detail.to_string()] {
        for forbidden in ["private-host", "base_url", "sk-private", "api_key", "owner_user_id"] {
            assert!(!body.contains(forbidden), "{forbidden} leaked in {body}");
        }
    }
}
