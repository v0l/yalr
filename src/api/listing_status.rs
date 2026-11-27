use std::sync::Arc;

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;

use crate::api::tenant::{internal, not_found, Scope, TenantError};
use crate::auth::admin::UserExtractor;
use crate::state::AppState;

#[derive(Serialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Listing {
    Listed,
    NeedsPrice,
    NeedsFullName,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ListedModel {
    pub id: String,
    pub status: Listing,
}

#[derive(Serialize, Debug)]
pub struct ListingReport {
    pub provider: String,
    pub public: bool,
    pub models: Vec<ListedModel>,
    pub short_names: Vec<String>,
}

pub fn classify(model: &str, owned: bool, priced: bool) -> Listing {
    if !model.contains('/') {
        Listing::NeedsFullName
    } else if owned && !priced {
        Listing::NeedsPrice
    } else {
        Listing::Listed
    }
}

pub async fn provider_listing(
    Path(slug): Path<String>,
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
) -> Result<Json<ListingReport>, TenantError> {
    let scope = Scope::from(user);
    let record = scope.provider(&state, &slug).await?;
    let live = state.config.router.get_providers().await;
    let provider = live.iter().find(|p| p.slug() == slug).ok_or_else(|| not_found("Provider not found"))?;
    let held = record.owner_user_id.is_some() && record.status != crate::db::APPROVED;
    let listed = match held {
        true => None,
        false => tokio::time::timeout(std::time::Duration::from_secs(5), provider.list_models()).await.ok().and_then(Result::ok),
    };
    let listed = match listed {
        Some(models) => models,
        None => state.model_cache.get(&slug).await.unwrap_or_default(),
    };
    let pricings = state.db.list_model_pricings().await.map_err(internal)?;
    let owned = record.owner_user_id.is_some();
    let models: Vec<ListedModel> = listed
        .into_iter()
        .map(|m| {
            let priced = pricings.iter().any(|r| r.provider_id == Some(record.id) && r.model_name.eq_ignore_ascii_case(&m.id));
            ListedModel { status: classify(&m.id, owned, priced), id: m.id }
        })
        .collect();
    let short_names = models.iter().filter(|m| m.status == Listing::NeedsFullName).map(|m| m.id.clone()).collect();
    Ok(Json(ListingReport {
        public: record.status == crate::db::APPROVED,
        provider: record.name,
        models,
        short_names,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_need_a_full_repo_id_and_unpriced_owned_models_need_a_price() {
        assert_eq!(classify("qwen3.8-27b", true, true), Listing::NeedsFullName);
        assert_eq!(classify("org/Qwen", true, false), Listing::NeedsPrice);
        assert_eq!(classify("org/Qwen", true, true), Listing::Listed);
        assert_eq!(classify("org/Qwen", false, false), Listing::Listed);
    }
}
