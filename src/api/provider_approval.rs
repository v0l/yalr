use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::api::tenant::Scope;
use crate::auth::admin::UserExtractor;
use crate::api::verification_view::{self, VerificationDetail, VerificationSummary};
use crate::db::{Provider, APPROVED, PENDING, SUSPENDED};
use crate::state::AppState;

type Failure = (StatusCode, String);
type ApiResult<T> = Result<Json<T>, Failure>;

#[derive(Deserialize)]
pub struct StatusRequest {
    pub action: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct ReferenceRequest {
    pub is_reference: bool,
}

#[derive(Serialize)]
pub struct ApprovalView {
    pub slug: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub status_locked: bool,
    pub is_reference: bool,
    pub verifications: Vec<VerificationSummary>,
}

fn internal(e: impl ToString) -> Failure {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn find(state: &AppState, slug: &str) -> Result<Provider, Failure> {
    state
        .db
        .get_provider_by_slug(slug)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, "Provider not found".to_string()))
}

async fn view(state: &AppState, provider: Provider) -> ApprovalView {
    ApprovalView {
        verifications: verification_view::latest(&state.db, provider.id).await,
        slug: provider.slug,
        status: provider.status,
        status_reason: provider.status_reason,
        status_locked: provider.status_locked,
        is_reference: provider.is_reference,
    }
}

pub async fn set_status(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Json(req): Json<StatusRequest>,
) -> ApiResult<ApprovalView> {
    let provider = find(&state, &slug).await?;
    let note = req.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let (status, reason, locked) = match req.action.as_str() {
        "approve" => (APPROVED, None, false),
        "suspend" => (SUSPENDED, Some(note.unwrap_or("Suspended by an admin").to_string()), true),
        "reject" => (SUSPENDED, Some(format!("Rejected: {}", note.unwrap_or("not accepted"))), true),
        "requeue" => (PENDING, None, false),
        other => return Err((StatusCode::BAD_REQUEST, format!("Unknown action '{other}'"))),
    };
    let updated = state
        .db
        .set_provider_status(provider.id, status, reason.as_deref(), locked)
        .await
        .map_err(internal)?;
    state.config.router.refresh_provider_access().await;
    Ok(Json(view(&state, updated).await))
}

pub async fn set_reference(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    Json(req): Json<ReferenceRequest>,
) -> ApiResult<ApprovalView> {
    let provider = find(&state, &slug).await?;
    if req.is_reference && provider.status != APPROVED {
        return Err((StatusCode::BAD_REQUEST, "Only an approved provider can be a reference".to_string()));
    }
    let updated = state.db.set_provider_reference(provider.id, req.is_reference).await.map_err(internal)?;
    Ok(Json(view(&state, updated).await))
}

pub async fn verify(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(slug): Path<String>,
) -> ApiResult<ApprovalView> {
    let scope = Scope::from(user);
    let provider = scope.provider(&state, &slug).await?;
    crate::api::abuse::owner_contact(
        &state,
        &scope,
        &provider,
        crate::verify::verify_and_apply(&state.db, &state.config.router, provider.id),
    )
    .await?;
    let fresh = scope.provider(&state, &slug).await?;
    Ok(Json(view(&state, fresh).await))
}

pub async fn verifications(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Path(slug): Path<String>,
) -> ApiResult<Vec<VerificationDetail>> {
    let provider = Scope::from(user).provider(&state, &slug).await?;
    Ok(Json(verification_view::history(&state.db, provider.id).await))
}
