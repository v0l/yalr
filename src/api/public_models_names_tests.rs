use std::sync::Arc;

use super::tenant_tests::tenancy;
use crate::db::{FiatPrices, NewModelPricing, NewProvider};

const MODEL: &str = "acme/model-1";

struct Stub;

#[async_trait::async_trait]
impl crate::providers::Provider for Stub {
    fn name(&self) -> &str {
        "named-p"
    }
    fn slug(&self) -> &str {
        "named-p"
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, crate::providers::ProviderError> {
        Ok(vec![crate::Model { id: MODEL.into(), object: "model".into(), created: 0, owned_by: "x".into() }])
    }
    async fn chat_completions(
        &self,
        _request: &crate::providers::ChatRequest,
    ) -> Result<crate::ChatCompletionResponse, crate::providers::ProviderError> {
        Err(crate::providers::ProviderError::Unsupported("stub".into()))
    }
    fn chat_completions_stream(
        &self,
        _request: &crate::providers::ChatRequest,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<crate::providers::StreamingChunk, crate::providers::ProviderError>>,
        crate::providers::ProviderError,
    > {
        Err(crate::providers::ProviderError::Unsupported("stub".into()))
    }
    async fn health_check(&self) -> Result<bool, crate::providers::ProviderError> {
        Ok(true)
    }
}

const PUBKEY: &str = "63fe6318dc58583cfe16810f86dd09e18bfd76aabc24a0081ce2856f330504ed";

async fn nostr_market() -> (super::tenant_tests::Tenancy, i64) {
    let t = tenancy().await;
    let db = &t.state.db;
    let owner = db.get_or_create_user_by_nostr_pubkey(PUBKEY).await.unwrap();
    let new = NewProvider { name: "named-p", slug: "named-p", base_url: "https://h.example.com/v1", api_key: None, provider_type: None };
    let created = db.create_owned_provider(owner.id, new).await.unwrap();
    let record = db.set_provider_status(created.id, "approved", None, false).await.unwrap();
    db.create_model_pricing(NewModelPricing {
        model_name: MODEL,
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: Some(500),
        price_per_1m_output_sats: Some(900),
        price_per_1m_cache_read_sats: None,
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: None,
        context_window: Some(32_768),
        max_output_tokens: Some(4_096),
        owner_user_id: Some(owner.id),
        provider_id: Some(record.id),
        currency: "sats",
        fiat: FiatPrices::default(),
    })
    .await
    .unwrap();
    t.state.config.router.add_provider(Arc::new(Stub)).await;
    t.state.config.router.refresh_full_name_index().await;
    (t, owner.id)
}

#[tokio::test]
async fn the_public_page_shows_a_short_npub_until_a_profile_is_known_and_never_the_username() {
    let (t, owner) = nostr_market().await;
    let (_, before) = t.get("", &format!("/api/public/models/{MODEL}")).await;
    let name = before["providers"][0]["owner"].as_str().unwrap().to_string();
    assert!(name.starts_with("npub1") && name.contains('…'), "{name}");
    assert!(!before.to_string().contains("nostr_"), "{before}");

    t.state.db.store_user_profile(owner, Some("Ripper Co"), None, None).await.unwrap();
    let fresh = crate::api::public_models::PublicCache::with_ttl(std::time::Duration::ZERO);
    let snapshot = fresh.snapshot(&t.state).await;
    let shown = serde_json::to_string(&snapshot.models[0].providers).unwrap();
    assert!(shown.contains("Ripper Co"), "{shown}");
    assert!(!shown.contains("nostr_") && !shown.contains(PUBKEY), "{shown}");
}

#[tokio::test]
async fn admin_lists_and_the_account_use_the_profile_name() {
    let (t, owner) = nostr_market().await;
    t.state.db.store_user_profile(owner, Some("Ripper Co"), Some("https://img.example/a.png"), None).await.unwrap();
    let (_, users) = t.get(&t.admin, "/api/users").await;
    let row = users.as_array().unwrap().iter().find(|u| u["id"] == owner).unwrap();
    assert_eq!(row["display_name"], "Ripper Co");
    assert_eq!(row["picture"], "https://img.example/a.png");
    let (_, providers) = t.get(&t.admin, "/api/providers").await;
    assert_eq!(providers["providers"][0]["owner_username"], "Ripper Co");
}
