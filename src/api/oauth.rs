//! Admin API endpoints for connecting subscription OAuth providers
//! (Claude Pro/Max, ChatGPT Plus/Pro).
//!
//! Flow:
//! 1. `POST /api/oauth/start` — returns an authorize URL; the admin opens it,
//!    authorizes, and copies back the resulting code (or full redirect URL).
//! 2. `POST /api/oauth/complete` — exchanges the code for tokens and creates the
//!    provider row, then reloads the router.
//! 3. `POST /api/oauth/:slug/reauth` — re-run the flow for an existing provider.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::db::{NewProvider, OAuthCredentials};
use crate::oauth::{
    self, generate_pkce, generate_state, OAuthKind, PendingLogin, PENDING_TTL,
};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct StartRequest {
    /// "anthropic" | "openai" (also accepts -oauth/claude/chatgpt aliases).
    pub provider: String,
    /// Display name for the new provider.
    pub name: String,
    /// URL-safe slug used for routing (e.g. "claude-max").
    pub slug: String,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub authorize_url: String,
    pub state: String,
    /// Human-readable hint about how to retrieve the code for this provider.
    pub instructions: String,
}

#[derive(Deserialize)]
pub struct CompleteRequest {
    pub state: String,
    /// Authorization code, `CODE#STATE`, or the full redirect URL.
    pub code: String,
}

#[derive(Serialize)]
pub struct CompleteResponse {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub provider_type: String,
}

#[derive(Deserialize)]
pub struct ReauthRequest {
    pub provider: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, String) {
    (status, msg.into())
}

/// Drop expired pending logins.
async fn prune(state: &AppState) {
    let mut pending = state.oauth_pending.lock().await;
    pending.retain(|_, p| p.created_at.elapsed() < PENDING_TTL);
}

fn instructions_for(kind: OAuthKind) -> &'static str {
    match kind {
        OAuthKind::Anthropic => {
            "Open the URL, approve access, then copy the code shown on the page \
             (it may look like CODE#STATE) and paste it back here."
        }
        OAuthKind::OpenAi => {
            "Open the URL and sign in with ChatGPT. Your browser will redirect to \
             a localhost URL that won't load — copy that full address bar URL \
             (it contains code & state) and paste it back here."
        }
    }
}

/// POST /api/oauth/start
#[axum::debug_handler]
pub async fn start(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<StartRequest>,
) -> Result<Json<StartResponse>, (StatusCode, String)> {
    let kind = OAuthKind::from_str(&req.provider)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown oauth provider"))?;

    if req.name.trim().is_empty() || req.slug.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name and slug are required"));
    }

    prune(&state).await;

    let pkce = generate_pkce();
    let csrf = generate_state();
    let authorize_url = oauth::build_authorize(kind, &pkce, &csrf);

    let pending = PendingLogin {
        kind,
        verifier: pkce.verifier,
        state: csrf.clone(),
        name: req.name.trim().to_string(),
        slug: req.slug.trim().to_lowercase().replace(' ', "-"),
        created_at: std::time::Instant::now(),
    };
    state.oauth_pending.lock().await.insert(csrf.clone(), pending);

    Ok(Json(StartResponse {
        authorize_url,
        state: csrf,
        instructions: instructions_for(kind).to_string(),
    }))
}

/// POST /api/oauth/complete
#[axum::debug_handler]
pub async fn complete(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<CompleteRequest>,
) -> Result<Json<CompleteResponse>, (StatusCode, String)> {
    let pending = {
        let mut store = state.oauth_pending.lock().await;
        store.remove(&req.state)
    }
    .ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown or expired login state"))?;

    if pending.created_at.elapsed() >= PENDING_TTL {
        return Err(err(StatusCode::BAD_REQUEST, "login expired, please restart"));
    }

    let (code, parsed_state) = oauth::parse_code_input(&req.code);
    if let Some(s) = &parsed_state {
        if s != &pending.state {
            return Err(err(StatusCode::BAD_REQUEST, "state mismatch"));
        }
    }

    let client = reqwest::Client::new();
    // Anthropic falls back to the verifier as state if none was returned.
    let exchange_state = parsed_state.unwrap_or_else(|| pending.verifier.clone());
    let tokens = oauth::exchange_code(&client, pending.kind, &code, &pending.verifier, &exchange_state)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("token exchange failed: {e}")))?;

    let created = persist_provider(&state, &pending, tokens).await?;

    state
        .config
        .router
        .reload_config()
        .await
        .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
            err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(created))
}

/// POST /api/oauth/:slug/reauth — start a fresh login bound to an existing provider's slug.
#[axum::debug_handler]
pub async fn reauth(
    State(state): State<std::sync::Arc<AppState>>,
    Path(slug): Path<String>,
    Json(req): Json<ReauthRequest>,
) -> Result<Json<StartResponse>, (StatusCode, String)> {
    let kind = OAuthKind::from_str(&req.provider)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "unknown oauth provider"))?;

    let existing = state
        .config
        .db
        .get_provider_by_slug(&slug)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "provider not found"))?;

    prune(&state).await;

    let pkce = generate_pkce();
    let csrf = generate_state();
    let authorize_url = oauth::build_authorize(kind, &pkce, &csrf);

    let pending = PendingLogin {
        kind,
        verifier: pkce.verifier,
        state: csrf.clone(),
        name: existing.name.clone(),
        slug: existing.slug.clone(),
        created_at: std::time::Instant::now(),
    };
    state.oauth_pending.lock().await.insert(csrf.clone(), pending);

    Ok(Json(StartResponse {
        authorize_url,
        state: csrf,
        instructions: instructions_for(kind).to_string(),
    }))
}

/// Create a new provider row or update the existing one (re-auth), with OAuth creds.
async fn persist_provider(
    state: &AppState,
    pending: &PendingLogin,
    tokens: oauth::OAuthTokens,
) -> Result<CompleteResponse, (StatusCode, String)> {
    let provider_type = pending.kind.provider_type();
    let creds = OAuthCredentials {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens.expires_at,
        account_id: tokens.account_id,
    };

    let existing = state
        .config
        .db
        .get_provider_by_slug(&pending.slug)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let provider = if let Some(p) = existing {
        state
            .config
            .db
            .update_provider_oauth(p.id, &creds)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        let new = NewProvider {
            name: &pending.name,
            slug: &pending.slug,
            base_url: pending.kind.default_base_url(),
            api_key: None,
            provider_type: Some(provider_type),
        };
        let created = state
            .config
            .db
            .create_provider(new)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        state
            .config
            .db
            .update_provider_oauth(created.id, &creds)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    Ok(CompleteResponse {
        id: provider.id,
        name: provider.name,
        slug: provider.slug,
        provider_type: provider.provider_type.as_str().to_string(),
    })
}
