use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::util::ServiceExt;

use super::tenant_tests::{tenancy_with_payments, Tenancy};
use crate::config::DefaultPricingConfig;
use crate::db::{FiatPrices, NewModelPricing, NewProvider};
use crate::providers::{ChatRequest, ListedModalities, Provider, ProviderError, StreamingChunk};
use crate::ChatCompletionResponse;

const MODEL: &str = "Piecrust/GLM-5.3-Flash-FP4-Abliterated";

struct Pool {
    name: String,
    vision: bool,
}

#[async_trait]
impl Provider for Pool {
    fn name(&self) -> &str {
        &self.name
    }
    fn slug(&self) -> &str {
        &self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![crate::Model { id: MODEL.into(), object: "model".into(), created: 0, owned_by: "x".into() }])
    }
    async fn listed_modalities(&self, _model: &str) -> Option<ListedModalities> {
        Some(ListedModalities {
            input_modalities: if self.vision { vec!["text".into(), "image".into()] } else { vec!["text".into()] },
            output_modalities: vec!["text".into()],
        })
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

struct Row {
    name: &'static str,
    vision: bool,
    input: i64,
    output: i64,
    request: i64,
    context: i32,
    max_output: i32,
}

async fn serve(t: &Tenancy, owner: i64, row: Row) {
    let db = &t.state.db;
    let new = NewProvider { name: row.name, slug: row.name, base_url: "https://h.example.com/v1", api_key: None, provider_type: None };
    let created = db.create_owned_provider(owner, new).await.unwrap();
    let record = db.set_provider_status(created.id, "approved", None, false).await.unwrap();
    db.create_model_pricing(NewModelPricing {
        model_name: MODEL,
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: Some(row.input),
        price_per_1m_output_sats: Some(row.output),
        price_per_1m_cache_read_sats: Some(row.input / 10),
        price_per_1m_cache_write_sats: Some(row.input / 10),
        price_per_request_sats: Some(row.request),
        context_window: Some(row.context),
        max_output_tokens: Some(row.max_output),
        owner_user_id: Some(owner),
        provider_id: Some(record.id),
        currency: "sats",
        fiat: FiatPrices::default(),
    })
    .await
    .unwrap();
    t.state.config.router.add_provider(Arc::new(Pool { name: row.name.into(), vision: row.vision })).await;
}

async fn production() -> Tenancy {
    let t = tenancy_with_payments(DefaultPricingConfig {
        price_per_1m_input_sats: 5,
        price_per_1m_output_sats: 9,
        price_per_request_sats: 0,
        context_window: 8192,
        max_output_tokens: 4096,
    })
    .await;
    let alice = t.state.db.get_user_by_username("alice").await.unwrap().unwrap();
    serve(&t, alice.id, Row { name: "dear", vision: true, input: 840, output: 840, request: 2, context: 512_000, max_output: 128_000 }).await;
    serve(&t, alice.id, Row { name: "cheap", vision: false, input: 420, output: 420, request: 1, context: 1_000_000, max_output: 256_000 }).await;
    t.state.config.router.refresh_full_name_index().await;
    t
}

pub(crate) async fn production_state() -> Arc<crate::state::AppState> {
    production().await.state
}

async fn anonymous(t: &Tenancy) -> Value {
    let response = t.app.clone().oneshot(Request::builder().uri("/v1/models").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn public_feed_shows_the_cheapest_provider_price_and_the_largest_limits() {
    let t = production().await;
    let feed = anonymous(&t).await;
    let m = &feed["data"][0];
    assert_eq!(m["id"], MODEL);
    assert_eq!(m["context_length"], 1_000_000);
    assert_eq!(m["top_provider"]["context_length"], 1_000_000);
    assert_eq!(m["top_provider"]["max_completion_tokens"], 256_000);
    assert_eq!(m["sats_pricing"]["prompt"], 0.00042);
    assert_eq!(m["sats_pricing"]["completion"], 0.00042);
    assert_eq!(m["sats_pricing"]["request"], 1.0);
    assert_eq!(m["sats_pricing"]["input_cache_read"], 0.000042);
    assert!(m["created"].as_i64().unwrap() > 1_600_000_000);
    assert_eq!(m["description"], "Served by 2 providers on YALR");
    assert_eq!(m["architecture"]["input_modalities"], serde_json::json!(["text", "image"]));
}

#[tokio::test]
async fn public_feed_only_claims_image_input_when_a_provider_lists_it() {
    let t = production().await;
    t.state.config.router.remove_provider("dear").await;
    t.state.config.router.refresh_full_name_index().await;
    let feed = anonymous(&t).await;
    assert_eq!(feed["data"][0]["architecture"]["input_modalities"], serde_json::json!(["text"]));
}

#[tokio::test]
async fn authenticated_list_carries_the_same_pool_data() {
    let t = production().await;
    let (status, list) = t.get(&t.alice, "/v1/models").await;
    assert_eq!(status, 200, "{list}");
    let m = list["data"].as_array().unwrap().iter().find(|m| m["id"] == MODEL).unwrap();
    assert_eq!(m["pricing"]["prompt"], 420);
    assert_eq!(m["pricing"]["request"], 1);
    assert_eq!(m["context_length"], 1_000_000);
    assert_eq!(m["max_output_tokens"], 256_000);
    assert!(m["created"].as_i64().unwrap() > 1_600_000_000);
    assert_eq!(m["architecture"]["input_modalities"], serde_json::json!(["text", "image"]));
}
