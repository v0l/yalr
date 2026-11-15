use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::tenant_tests::{tenancy_with_payments, Tenancy};
use crate::config::DefaultPricingConfig;
use crate::db::{FiatPrices, NewModelPricing, NewProvider, NewRoutingConfig, Record};
use crate::providers::{ChatRequest, Provider, ProviderError, StreamingChunk};
use crate::ChatCompletionResponse;

const HOUR_MS: i64 = 3_600_000;

struct Listing {
    name: String,
    models: Vec<&'static str>,
}

#[async_trait]
impl Provider for Listing {
    fn name(&self) -> &str {
        &self.name
    }
    fn slug(&self) -> &str {
        &self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(self
            .models
            .iter()
            .map(|id| crate::Model { id: (*id).into(), object: "model".into(), created: 0, owned_by: "x".into() })
            .collect())
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

async fn provider(t: &Tenancy, owner: Option<i64>, name: &str, models: Vec<&'static str>) -> (Arc<dyn Provider>, i64) {
    let new = NewProvider { name, slug: name, base_url: "https://h.example.com/v1", api_key: None, provider_type: None };
    let created = match owner {
        Some(o) => t.state.db.create_owned_provider(o, new).await.unwrap(),
        None => t.state.db.create_provider(new).await.unwrap(),
    };
    let record = t.state.db.set_provider_status(created.id, "approved", None, false).await.unwrap();
    let live: Arc<dyn Provider> = Arc::new(Listing { name: name.into(), models });
    t.state.config.router.add_provider(live.clone()).await;
    (live, record.id)
}

async fn price(t: &Tenancy, model: &str, provider_id: i64, owner: i64) {
    t.state
        .db
        .create_model_pricing(NewModelPricing {
            model_name: model,
            is_advertised: true,
            is_free: false,
            price_per_1m_input_sats: Some(700),
            price_per_1m_output_sats: Some(1_100),
            price_per_1m_cache_read_sats: None,
            price_per_1m_cache_write_sats: None,
            price_per_request_sats: None,
            context_window: Some(65_536),
            max_output_tokens: None,
            owner_user_id: Some(owner),
            provider_id: Some(provider_id),
            currency: "sats",
            fiat: FiatPrices::default(),
        })
        .await
        .unwrap();
}

fn request(id: &str, model: &str, provider: &str, ttft: i64, total: i64, tokens: i64) -> Record {
    Record {
        request_id: id.into(),
        started_at_ms: chrono::Utc::now().timestamp_millis() - HOUR_MS,
        endpoint: "/v1/chat/completions".into(),
        model: model.into(),
        provider: Some(provider.into()),
        status: "ok".into(),
        attempts: 1,
        first_event_ms: Some(ttft),
        total_ms: total,
        completion_tokens: Some(tokens),
        ..Default::default()
    }
}

async fn production() -> Tenancy {
    let t = tenancy_with_payments(DefaultPricingConfig {
        price_per_1m_input_sats: 500,
        price_per_1m_output_sats: 900,
        price_per_request_sats: 0,
        context_window: 8192,
        max_output_tokens: 4096,
    })
    .await;
    let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
    let (admin, _) = provider(&t, None, "openrouter", vec!["org/used", "org/idle", "short-served"]).await;
    let (_, priced) = provider(&t, Some(alice.id), "priced-p", vec!["org/Priced"]).await;
    price(&t, "org/Priced", priced, alice.id).await;
    provider(&t, Some(alice.id), "ripper", vec!["orca/Unpriced-27B"]).await;
    provider(&t, Some(alice.id), "spark", vec!["qwen3.8-27b"]).await;
    t.state
        .db
        .create_routing_config(NewRoutingConfig {
            name: "classify".into(),
            strategy: "round_robin".into(),
            health_check_enabled: false,
            health_check_interval_seconds: 30,
            health_check_timeout_seconds: 5,
        })
        .await
        .unwrap();
    t.state.config.router.register_route("classify", vec![admin]).await;
    t.state
        .db
        .insert_request_logs(&[
            request("c1", "classify", "openrouter", 100, 1_100, 100),
            request("c2", "classify", "openrouter", 300, 2_300, 400),
            request("u1", "org/used", "openrouter", 200, 1_200, 100),
        ])
        .await
        .unwrap();
    t.state.config.router.refresh_full_name_index().await;
    t
}

fn ids(list: &Value) -> Vec<&str> {
    list["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect()
}

fn model<'a>(list: &'a Value, id: &str) -> &'a Value {
    list["models"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("{id} missing in {list}"))
}

#[tokio::test]
async fn the_default_list_shows_served_names_with_traffic_and_hides_idle_pass_throughs() {
    let t = production().await;
    let (status, list) = t.get("", "/api/public/models").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(ids(&list), vec!["classify", "org/Priced", "org/used"]);

    let classify = model(&list, "classify");
    assert_eq!(classify["kind"], "routed");
    assert_eq!(classify["requests_24h"], 2);
    assert_eq!(classify["price_ranges"][0]["currency"], "sats");
    assert_eq!(classify["price_ranges"][0]["min_input"], 500.0);
    assert_eq!(classify["price_ranges"][0]["max_output"], 900.0);
    assert_eq!(classify["cheapest_provider"], "openrouter");

    let used = model(&list, "org/used");
    assert_eq!(used["requests_24h"], 1);
    assert_eq!(used["price_ranges"][0]["min_input"], 500.0);
    assert_eq!(model(&list, "org/Priced")["context_length"], 65_536);
}

#[tokio::test]
async fn show_all_lists_every_pass_through_model() {
    let t = production().await;
    let (_, all) = t.get("", "/api/public/models?all=true").await;
    assert_eq!(ids(&all), vec!["classify", "org/Priced", "org/idle", "org/used"]);
    assert_eq!(model(&all, "org/idle")["requests_24h"], 0);
    assert_eq!(model(&all, "org/idle")["price_ranges"][0]["min_input"], 500.0);
}

#[tokio::test]
async fn short_name_traffic_is_credited_to_the_serving_provider() {
    let t = production().await;
    let (status, detail) = t.get("", "/api/public/models/classify").await;
    assert_eq!(status, 200, "{detail}");
    let provider = &detail["providers"][0];
    assert_eq!(provider["name"], "openrouter");
    assert_eq!(provider["requests_24h"], 2);
    assert_eq!(provider["ttft_p50_ms"], 100.0);
    assert_eq!(provider["price"]["input_per_1m"], 500.0);
    assert_eq!(detail["fastest_provider"], "openrouter");
}

#[tokio::test]
async fn an_owner_sees_which_models_need_a_price_or_a_full_repo_id() {
    let t = production().await;
    let (status, ripper) = t.get(&t.alice, "/api/providers/ripper/listing").await;
    assert_eq!(status, 200, "{ripper}");
    assert_eq!(ripper["models"][0]["id"], "orca/Unpriced-27B");
    assert_eq!(ripper["models"][0]["status"], "needs_price");
    assert_eq!(ripper["short_names"].as_array().unwrap().len(), 0);

    let (_, spark) = t.get(&t.alice, "/api/providers/spark/listing").await;
    assert_eq!(spark["models"][0]["status"], "needs_full_name");
    assert_eq!(spark["short_names"][0], "qwen3.8-27b");

    let (_, priced) = t.get(&t.alice, "/api/providers/priced-p/listing").await;
    assert_eq!(priced["models"][0]["status"], "listed");
    assert_eq!(t.get(&t.bob, "/api/providers/spark/listing").await.0, 404);

    let (_, public) = t.get("", "/api/public/models?all=true").await;
    assert!(!ids(&public).contains(&"orca/Unpriced-27B") && !ids(&public).contains(&"qwen3.8-27b"));
}
