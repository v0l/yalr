use crate::api::health::{build_provider_health_entry, ProviderHealthEntry};
use crate::api::tenant::{bad_request, internal, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::db::{Provider, ProviderType};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

type ApiResult<T> = Result<Json<T>, TenantError>;

#[derive(Serialize)]
pub struct ProviderResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub provider_type: String,
    pub owner_user_id: Option<i64>,
    pub owner_username: Option<String>,
    pub has_api_key: bool,
    pub status: String,
    pub status_reason: Option<String>,
    pub status_locked: bool,
    pub is_reference: bool,
    pub max_concurrency: Option<i32>,
    pub ttft_deadline_ms: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort_map: Option<crate::providers::effort::EffortMap>,
    pub verifications: Vec<crate::api::verification_view::VerificationSummary>,
    pub created_at: crate::db::Timestamp,
    pub updated_at: crate::db::Timestamp,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<ProviderHealthEntry>,
    pub payment_options: Vec<PaymentOption>,
    pub is_oauth: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_client_version: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<crate::db::ExtraBody>,
}

#[derive(Serialize)]
pub struct OAuthStatus {
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
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
    pub created_at: crate::db::Timestamp,
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
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub provider_type: String,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ProviderUpdateRequest {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub provider_type: Option<String>,
    pub client_version: Option<String>,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub owner_user_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub max_concurrency: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub ttft_deadline_ms: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present_or_null")]
    pub reasoning_effort_map: Option<Option<serde_json::Value>>,
}

fn present_or_null<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn payment_options(kind: ProviderType) -> Vec<PaymentOption> {
    match kind {
        ProviderType::Routstr => vec![PaymentOption {
            currency: SupportedCurrencyType::Sats,
            payment_method: PaymentMethod::Lightning,
        }],
        ProviderType::Ppq => vec![PaymentOption {
            currency: SupportedCurrencyType::UsdMicro,
            payment_method: PaymentMethod::Lightning,
        }],
        _ => vec![],
    }
}

fn oauth_status(p: &Provider) -> Option<OAuthStatus> {
    if !p.provider_type.is_oauth() {
        return None;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    Some(OAuthStatus {
        connected: p.oauth_access_token.is_some() && p.oauth_refresh_token.is_some(),
        expires_at: p.oauth_expires_at,
        expired: p.oauth_expires_at.is_some_and(|e| e <= now_ms),
    })
}

fn response(p: Provider, owner_username: Option<String>, health: Option<ProviderHealthEntry>) -> ProviderResponse {
    ProviderResponse {
        status: p.status.clone(),
        status_reason: p.status_reason.clone(),
        status_locked: p.status_locked,
        is_reference: p.is_reference,
        max_concurrency: p.max_concurrency,
        ttft_deadline_ms: p.ttft_deadline_ms,
        reasoning_effort_map: crate::providers::effort::parse_stored(p.reasoning_effort_map.as_deref()),
        verifications: vec![],
        extra_body: p.extra_body_map(),
        has_api_key: p.api_key.as_deref().is_some_and(|k| !k.is_empty()),
        payment_options: payment_options(p.provider_type),
        is_oauth: p.provider_type.is_oauth(),
        oauth: oauth_status(&p),
        default_client_version: crate::providers::default_client_version(p.provider_type),
        id: p.id,
        name: p.name,
        slug: p.slug,
        base_url: p.base_url,
        provider_type: p.provider_type.as_str().to_string(),
        owner_user_id: p.owner_user_id,
        owner_username,
        created_at: p.created_at,
        updated_at: p.updated_at,
        health,
        client_version: p.client_version,
    }
}

async fn owner_username(state: &AppState, owner: Option<i64>) -> Option<String> {
    state.db.provider_owner_username(owner?).await.ok().flatten()
}

pub(crate) async fn reload(state: &AppState) -> Result<(), TenantError> {
    state.config.router.reload_config().await.map_err(|e| internal(e))
}

pub async fn list_providers(State(state): State<Arc<AppState>>, user: UserExtractor) -> ApiResult<ListProvidersResponse> {
    let scope = Scope::from(user);
    let providers = scope.providers(&state).await?;
    let mut health_by_slug: HashMap<String, ProviderHealthEntry> = HashMap::new();
    for provider in state.config.router.get_providers().await {
        if providers.iter().any(|p| p.slug == provider.slug()) {
            health_by_slug.insert(provider.slug().to_string(), build_provider_health_entry(&state, provider.as_ref()).await);
        }
    }
    let mut owner_names: HashMap<i64, Option<String>> = HashMap::new();
    for owner in providers.iter().filter_map(|p| p.owner_user_id) {
        if !owner_names.contains_key(&owner) {
            owner_names.insert(owner, owner_username(&state, Some(owner)).await);
        }
    }
    let mut out = Vec::with_capacity(providers.len());
    for p in providers {
        let name = p.owner_user_id.and_then(|id| owner_names.get(&id).cloned().flatten());
        let health = health_by_slug.remove(&p.slug);
        let verifications = crate::api::verification_view::latest(&state.db, p.id).await;
        out.push(ProviderResponse { verifications, ..response(p, name, health) });
    }
    let providers = out;
    Ok(Json(ListProvidersResponse { providers }))
}

pub async fn create_provider(
    State(state): State<Arc<AppState>>,
    peer: Option<axum::extract::ConnectInfo<std::net::SocketAddr>>,
    headers: axum::http::HeaderMap,
    user: UserExtractor,
    Json(request): Json<ProviderCreateRequest>,
) -> ApiResult<ProviderCreateResponse> {
    let scope = Scope::from(user);
    scope.check_url(&state, &request.base_url).await?;
    let provider_type = scope.provider_type(&request.provider_type)?;
    let slug = scope.provider_slug(&request.slug)?;
    let extra_body = crate::api::shared::parse_extra_body(request.extra_body.clone())?;
    if let Some(owner) = scope.owner() {
        let (recent, pending) = state.db.owned_provider_counts(owner).await.map_err(internal)?;
        if recent >= crate::api::abuse::CREATES_PER_DAY as i64 {
            return Err((StatusCode::TOO_MANY_REQUESTS, "You can add 5 providers a day, try again tomorrow".to_string()));
        }
        if pending >= crate::api::abuse::PENDING_PER_USER {
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                "You have 3 providers waiting for approval. Verify or delete one first".to_string(),
            ));
        }
        let ip = crate::api::abuse::client_ip(&headers, peer.map(|p| p.0));
        state.config.router.abuse_limits.create(owner, &ip)?;
    }
    let name = match scope.owner() {
        None => request.name.trim().to_string(),
        Some(_) => format!("{}/{}", scope.handle(), request.name.trim()),
    };
    let new_provider = crate::db::NewProvider {
        name: &name,
        slug: &slug,
        base_url: request.base_url.trim(),
        api_key: Some(request.api_key.as_str()).filter(|k| !k.is_empty()),
        provider_type: Some(provider_type),
    };
    let created = match scope.owner() {
        None => state.db.create_provider(new_provider).await,
        Some(owner) => state.db.create_owned_provider(owner, new_provider).await,
    }
    .map_err(|e| match e {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            (StatusCode::CONFLICT, "A provider with that name or slug exists".to_string())
        }
        other => internal(other),
    })?;
    if let Some(body) = &extra_body {
        state.db.set_provider_extra_body(created.id, Some(body)).await.map_err(internal)?;
    }
    reload(&state).await?;
    tracing::info!(provider = %created.name, owner = ?created.owner_user_id, "Provider added");
    Ok(Json(ProviderCreateResponse {
        id: created.id,
        name: created.name,
        slug: created.slug,
        base_url: created.base_url,
        created_at: created.created_at,
    }))
}

pub async fn delete_provider(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
) -> ApiResult<ProviderDeleteResponse> {
    let provider = Scope::from(user).provider(&state, &slug).await?;
    state.db.delete_provider(provider.id).await.map_err(internal)?;
    reload(&state).await?;
    tracing::info!(provider_slug = slug, "Provider deleted");
    Ok(Json(ProviderDeleteResponse { deleted: true, slug }))
}

pub async fn update_provider(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(request): Json<ProviderUpdateRequest>,
) -> ApiResult<ProviderResponse> {
    let scope = Scope::from(user);
    let provider = scope.provider(&state, &slug).await?;
    if let Some(url) = request.base_url.as_deref() {
        scope.check_url(&state, url).await?;
    }
    let provider_type = match request.provider_type.as_deref() {
        Some(raw) => Some(scope.provider_type(raw)?),
        None => None,
    };
    if !scope.is_admin() && (request.slug.is_some() || request.owner_user_id.is_some()) {
        return Err(bad_request("Only admins can change a provider's slug or owner"));
    }
    let extra_body = crate::api::shared::parse_extra_body(request.extra_body.clone())?;
    let updated = state
        .db
        .update_provider(
            provider.id,
            crate::db::UpdateProvider {
                name: request.name.as_deref(),
                slug: request.slug.as_deref(),
                base_url: request.base_url.as_deref(),
                api_key: request.api_key.as_deref().filter(|k| !k.is_empty()).map(Some),
                provider_type,
            },
        )
        .await
        .map_err(internal)?;
    let updated = match request.max_concurrency {
        Some(limit) => {
            let limit = limit.filter(|l| *l > 0);
            state.db.set_provider_max_concurrency(updated.id, limit).await.map_err(internal)?;
            state.db.get_provider_by_id(updated.id).await.map_err(internal)?.unwrap_or(updated)
        }
        None => updated,
    };
    let updated = if request.ttft_deadline_ms.is_some() || request.reasoning_effort_map.is_some() {
        let deadline = match request.ttft_deadline_ms {
            Some(value) => value.filter(|ms| *ms > 0),
            None => updated.ttft_deadline_ms,
        };
        let efforts = match request.reasoning_effort_map.clone() {
            Some(value) => crate::providers::effort::validate(value.unwrap_or(serde_json::Value::Null))
                .map_err(|e| bad_request(&e))?,
            None => crate::providers::effort::parse_stored(updated.reasoning_effort_map.as_deref()),
        };
        state.db.set_provider_tuning(updated.id, deadline, efforts.as_ref()).await.map_err(internal)?;
        state.db.get_provider_by_id(updated.id).await.map_err(internal)?.unwrap_or(updated)
    } else {
        updated
    };
    let updated = match request.client_version.as_deref() {
        Some(version) => {
            let version = Some(version.trim()).filter(|v| !v.is_empty());
            state.db.set_provider_client_version(updated.id, version).await.map_err(internal)?
        }
        None => updated,
    };
    let updated = match request.owner_user_id {
        Some(owner) => {
            if let Some(id) = owner {
                state
                    .db
                    .get_user_by_id(id)
                    .await
                    .map_err(internal)?
                    .ok_or_else(|| bad_request(&format!("User {id} not found")))?;
            }
            state.db.set_provider_owner(updated.id, owner).await.map_err(internal)?
        }
        None => updated,
    };
    let updated = if request.extra_body.is_some() {
        state.db.set_provider_extra_body(updated.id, extra_body.as_ref()).await.map_err(internal)?
    } else {
        updated
    };
    reload(&state).await?;
    tracing::info!(provider_slug = slug, "Provider updated");
    let changed = request.base_url.as_deref().is_some_and(|url| url != provider.base_url)
        || request.api_key.as_deref().is_some_and(|k| !k.is_empty())
        || provider_type.is_some_and(|k| k != provider.provider_type);
    if changed {
        crate::verify::mark_changed(&state.db, &state.config.router, &updated).await;
    }
    let updated = state.db.get_provider_by_id(updated.id).await.map_err(internal)?.unwrap_or(updated);
    let name = owner_username(&state, updated.owner_user_id).await;
    let verifications = crate::api::verification_view::latest(&state.db, updated.id).await;
    Ok(Json(ProviderResponse { verifications, ..response(updated, name, None) }))
}
