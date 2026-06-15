use crate::providers::CurrencyAmount;
use crate::state::AppState;
use crate::api::health::{build_provider_health_entry, ProviderHealthEntry};
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub provider_type: String,
    pub created_at: String,
    pub updated_at: String,
    /// Live health+metrics for this provider.
    /// Only present when the router has the provider loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ProviderHealthEntry>,
    /// Supported payment options for this provider.
    pub payment_options: Vec<PaymentOption>,
    /// Whether this provider authenticates via OAuth (subscription) rather than an API key.
    pub is_oauth: bool,
    /// OAuth connection status (present only for OAuth providers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthStatus>,
}

#[derive(Serialize)]
pub struct OAuthStatus {
    /// True when an access/refresh token pair is stored.
    pub connected: bool,
    /// Unix epoch milliseconds at which the access token expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// True when the stored access token is already past its expiry.
    pub expired: bool,
}

#[derive(Serialize)]
pub struct PaymentOption {
    pub currency: SupportedCurrencyType,
    pub payment_method: PaymentMethod,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethod {
    Lightning,
    Redirect,
    Manual,
    PaymentLink,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportedCurrencyType {
    Msats,
    Sats,
    UsdMicro,
}

#[derive(Serialize)]
pub struct ListProvidersResponse {
    pub providers: Vec<ProviderResponse>,
}

#[derive(Serialize)]
pub struct ProviderCreateResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ProviderDeleteResponse {
    pub deleted: bool,
    pub slug: String,
}

#[derive(Deserialize)]
pub struct ProviderCreateRequest {
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: String,
    pub provider_type: String,
}

#[derive(Deserialize)]
pub struct ProviderUpdateRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub provider_type: Option<String>,
}

#[axum::debug_handler]
pub async fn list_providers(State(state): State<std::sync::Arc<AppState>>) -> Json<ListProvidersResponse> {
    let providers = match state.config.db.list_providers().await {
        Ok(providers) => providers,
        Err(e) => {
            tracing::error!("Failed to list providers from DB: {}", e);
            return Json(ListProvidersResponse { providers: vec![] });
        }
    };

    // Build slug→health-entry map from live router providers
    let live_providers = state.config.router.get_providers().await;
    let mut health_by_slug: std::collections::HashMap<String, ProviderHealthEntry> =
        std::collections::HashMap::new();
    for provider in &live_providers {
        let entry = build_provider_health_entry(&state, provider.as_ref()).await;
        health_by_slug.insert(provider.slug().to_string(), entry);
    }

    let providers_list: Vec<ProviderResponse> = providers
        .into_iter()
        .map(|p| {
            // Determine supported currencies based on provider type
            let supported_currencies = match p.provider_type {
                crate::db::ProviderType::Routstr => vec![PaymentOption {
                    currency: SupportedCurrencyType::Sats,
                    payment_method: PaymentMethod::Lightning,
                }],
                crate::db::ProviderType::Ppq => vec![PaymentOption {
                    currency: SupportedCurrencyType::UsdMicro,
                    payment_method: PaymentMethod::Lightning,
                }],
                _ => vec![], // Other providers don't support direct top-ups
            };

            let is_oauth = p.provider_type.is_oauth();
            let oauth = if is_oauth {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                Some(OAuthStatus {
                    connected: p.oauth_access_token.is_some() && p.oauth_refresh_token.is_some(),
                    expires_at: p.oauth_expires_at,
                    expired: p.oauth_expires_at.map(|e| e <= now_ms).unwrap_or(false),
                })
            } else {
                None
            };

            ProviderResponse {
                id: p.id,
                name: p.name.clone(),
                slug: p.slug.clone(),
                base_url: p.base_url,
                provider_type: p.provider_type.as_str().to_string(),
                created_at: p.created_at,
                updated_at: p.updated_at,
                health: health_by_slug.remove(&p.slug),
                payment_options: supported_currencies,
                is_oauth,
                oauth,
            }
        })
        .collect();

    Json(ListProvidersResponse {
        providers: providers_list,
    })
}

#[axum::debug_handler]
pub async fn create_provider(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<ProviderCreateRequest>,
) -> Result<Json<ProviderCreateResponse>, (axum::http::StatusCode, String)> {
    use crate::db::ProviderType;

    let provider_type = ProviderType::from_str(&request.provider_type)
        .unwrap_or(ProviderType::OpenAi);

    let new_provider = crate::db::NewProvider {
        name: &request.name,
        slug: &request.slug,
        base_url: &request.base_url,
        api_key: Some(&request.api_key),
        provider_type: Some(provider_type),
    };

    let created = state.config.db.create_provider(new_provider).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        provider_name = request.name,
        "Provider added and config reloaded successfully"
    );

    Ok(Json(ProviderCreateResponse {
        id: created.id,
        name: created.name,
        slug: created.slug,
        base_url: created.base_url,
        created_at: created.created_at,
    }))
}

/// POST /providers/:slug/generate-api-key — Generate API key for PPQ/Routstr providers
///
/// For PPQ providers, creates a new account and returns a fresh API key.
/// For Routstr providers, this is not supported (Routstr requires external key setup).
#[axum::debug_handler]
pub async fn generate_provider_api_key(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    use crate::db::ProviderType;
    use reqwest::Client;

    // Get the provider
    let provider = state.config.db.get_provider_by_slug(&slug).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", slug)))?;

    // Check if it's a supported provider type
    if provider.provider_type != ProviderType::Ppq {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "API key generation is only supported for PPQ providers".to_string(),
        ));
    }

    // Call PPQ's /accounts/create endpoint to generate a new API key
    let client = Client::new();
    let response = client
        .post("https://api.ppq.ai/accounts/create")
        .send()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to contact PPQ: {}", e)))?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("PPQ account creation failed: {}", error_text),
        ));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid PPQ response: {}", e)))?;

    // Extract the API key and credit_id from the response
    let api_key = body
        .get("data")
        .and_then(|d| d.get("api_key"))
        .and_then(|k| k.as_str())
        .ok_or_else(|| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "PPQ did not return an API key".to_string()))?;

    let credit_id = body
        .get("data")
        .and_then(|d| d.get("credit_id"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    // Update the provider with the new API key
    let updated = state.config.db.update_provider(provider.id, crate::db::UpdateProvider {
        name: None,
        slug: None,
        base_url: None,
        api_key: Some(Some(api_key)),
        provider_type: None,
    }).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Reload config
    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Generated new PPQ API key");

    // Return the API key and credit_id to the client
    Ok(Json(serde_json::json!({
        "api_key": api_key,
        "credit_id": credit_id,
        "provider": {
            "id": updated.id,
            "name": updated.name,
            "slug": updated.slug,
            "base_url": updated.base_url,
            "provider_type": updated.provider_type.as_str(),
        }
    })))
}

/// POST /providers/:slug/topup — Create top-up request for any provider
///
/// Returns a generic PaymentInstruction enum that tells the admin UI how to handle the payment.
#[axum::debug_handler]
pub async fn create_provider_topup(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(body): Json<crate::payments::instructions::TopupRequest>,
) -> Result<Json<crate::payments::instructions::TopupResponse>, (axum::http::StatusCode, String)> {
    // Get the provider
    let provider = state.config.db.get_provider_by_slug(&slug).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", slug)))?;

    if body.amount <= 0 {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "Amount must be greater than 0".to_string(),
        ));
    }

    // Create top-up via provider trait method
    let providers = state.config.router.get_providers().await;
    let provider_arc = providers.into_iter().find(|p| p.slug() == slug)
        .ok_or_else(|| (axum::http::StatusCode::BAD_REQUEST, "Provider not found in router".to_string()))?;

    // Convert to CurrencyAmount based on currency type
    let amount = match body.currency {
        crate::payments::instructions::CurrencyType::Sats => CurrencyAmount::Sats(body.amount),
        crate::payments::instructions::CurrencyType::Msats => CurrencyAmount::Msats(body.amount),
        crate::payments::instructions::CurrencyType::UsdMicro => CurrencyAmount::UsdMicro(body.amount),
    };

    let instruction = provider_arc.create_topup(amount).await
        .ok_or_else(|| {
            tracing::error!(
                provider_slug = slug,
                provider_type = provider.provider_type.as_str(),
                "Provider does not support top-ups or top-up failed"
            );
            (axum::http::StatusCode::BAD_REQUEST, format!("Provider '{}' does not support top-ups or the top-up request failed. Check server logs for details.", provider.name))
        })?;

    Ok(Json(crate::payments::instructions::TopupResponse {
        provider: crate::payments::instructions::ProviderInfo {
            slug: provider.slug.clone(),
            name: provider.name.clone(),
        },
        instruction,
        message: Some(format!("Complete payment to top up {} balance", provider.name)),
    }))
}

pub async fn delete_provider(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<ProviderDeleteResponse>, (axum::http::StatusCode, String)> {
    tracing::info!(provider_slug = slug, "Deleting provider");

    // Delete from database
    sqlx::query("DELETE FROM providers WHERE slug = ?")
        .bind(&slug)
        .execute(&state.config.db.pool)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Provider deleted and config reloaded");

    Ok(Json(ProviderDeleteResponse {
        deleted: true,
        slug,
    }))
}

#[axum::debug_handler]
pub async fn update_provider(
    Path(slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<ProviderUpdateRequest>,
) -> Result<Json<ProviderResponse>, (axum::http::StatusCode, String)> {
    use crate::db::ProviderType;

    let provider = state.config.db.get_provider_by_slug(&slug).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", slug)))?;

    let provider_type = request.provider_type.as_deref()
        .and_then(ProviderType::from_str);

    let api_key = request.api_key.as_deref().map(Some);

    let updated = state.config.db.update_provider(provider.id, crate::db::UpdateProvider {
        name: request.name.as_deref(),
        slug: request.slug.as_deref(),
        base_url: request.base_url.as_deref(),
        api_key,
        provider_type,
    }).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    state.config.router.reload_config().await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(provider_slug = slug, "Provider updated and config reloaded");

    // Determine supported currencies based on provider type
    let payment_options = match updated.provider_type {
        crate::db::ProviderType::Routstr => vec![PaymentOption {
            currency: SupportedCurrencyType::Sats,
            payment_method: PaymentMethod::Lightning,
        }],
        crate::db::ProviderType::Ppq => vec![PaymentOption {
            currency: SupportedCurrencyType::UsdMicro,
            payment_method: PaymentMethod::Lightning,
        }],
        _ => vec![],
    };

    Ok(Json(ProviderResponse {
        id: updated.id,
        name: updated.name,
        slug: updated.slug,
        base_url: updated.base_url,
        provider_type: updated.provider_type.as_str().to_string(),
        created_at: updated.created_at,
        updated_at: updated.updated_at,
        health: None,
        payment_options,
        is_oauth: updated.provider_type.is_oauth(),
        oauth: None,
    }))
}
