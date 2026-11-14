use crate::api::tenant::{bad_request, internal, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::ProviderType;
use crate::payments::instructions::{CurrencyType, ProviderInfo, TopupRequest, TopupResponse};
use crate::providers::CurrencyAmount;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
struct PpqAccount {
    data: PpqAccountData,
}

#[derive(Deserialize)]
struct PpqAccountData {
    api_key: String,
    #[serde(default)]
    credit_id: String,
}

#[derive(Serialize)]
pub struct GeneratedKeyProvider {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub provider_type: &'static str,
}

#[derive(Serialize)]
pub struct GeneratedKeyResponse {
    pub api_key: String,
    pub credit_id: String,
    pub provider: GeneratedKeyProvider,
}

pub async fn generate_provider_api_key(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<GeneratedKeyResponse>, TenantError> {
    let provider = Scope::from(user).provider(&state, &slug).await?;
    if provider.provider_type != ProviderType::Ppq {
        return Err(bad_request("API key generation is only supported for PPQ providers"));
    }
    let response = reqwest::Client::new()
        .post("https://api.ppq.ai/accounts/create")
        .send()
        .await
        .map_err(|e| internal(format!("Failed to contact PPQ: {e}")))?;
    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(bad_request(&format!("PPQ account creation failed: {error_text}")));
    }
    let account: PpqAccount = response.json().await.map_err(|e| internal(format!("Invalid PPQ response: {e}")))?;
    let updated = state
        .db
        .update_provider(
            provider.id,
            crate::db::UpdateProvider {
                name: None,
                slug: None,
                base_url: None,
                api_key: Some(Some(&account.data.api_key)),
                provider_type: None,
            },
        )
        .await
        .map_err(internal)?;
    crate::api::providers::reload(&state).await?;
    tracing::info!(provider_slug = slug, "Generated new PPQ API key");
    Ok(Json(GeneratedKeyResponse {
        api_key: account.data.api_key,
        credit_id: account.data.credit_id,
        provider: GeneratedKeyProvider {
            id: updated.id,
            name: updated.name,
            slug: updated.slug,
            base_url: updated.base_url,
            provider_type: updated.provider_type.as_str(),
        },
    }))
}

pub async fn create_provider_topup(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(body): Json<TopupRequest>,
) -> Result<Json<TopupResponse>, TenantError> {
    let provider = Scope::from(user).provider(&state, &slug).await?;
    if body.amount <= 0 {
        return Err(bad_request("Amount must be greater than 0"));
    }
    let live = state
        .config
        .router
        .get_providers()
        .await
        .into_iter()
        .find(|p| p.slug() == slug)
        .ok_or_else(|| bad_request("Provider not found in router"))?;
    let amount = match body.currency {
        CurrencyType::Sats => CurrencyAmount::Sats(body.amount),
        CurrencyType::Msats => CurrencyAmount::Msats(body.amount),
        CurrencyType::UsdMicro => CurrencyAmount::UsdMicro(body.amount),
    };
    let instruction = live.create_topup(amount).await.ok_or_else(|| {
        tracing::error!(provider_slug = slug, provider_type = provider.provider_type.as_str(), "Top-up failed");
        bad_request(&format!(
            "Provider '{}' does not support top-ups or the top-up request failed. Check server logs for details.",
            provider.name
        ))
    })?;
    Ok(Json(TopupResponse {
        provider: ProviderInfo { slug: provider.slug.clone(), name: provider.name.clone() },
        instruction,
        message: Some(format!("Complete payment to top up {} balance", provider.name)),
    }))
}
