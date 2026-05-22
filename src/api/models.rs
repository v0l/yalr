use crate::router::{DbModelInfo, ModelInfoDetector};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Extension,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A model entry conforming to RIP-01 / RIP-05: includes pricing in sats.
#[derive(Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
    /// Pricing structure per RIP-05. None if payments are disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
}

/// Pricing breakdown per RIP-05, in sats.
#[derive(Serialize)]
pub struct ModelPricing {
    /// Sats per 1M input/prompt tokens.
    pub prompt: i64,
    /// Sats per 1M output/completion tokens.
    pub completion: i64,
    /// Sats per request.
    pub request: i64,
    /// Unit for token pricing (always "1M tokens").
    pub unit: String,
}

#[derive(Serialize)]
pub struct ModelsListResponse {
    pub object: String,
    pub data: Vec<ModelEntry>,
}

#[derive(Serialize)]
pub struct SyncModelsResponse {
    pub provider: String,
    pub models: Vec<serde_json::Value>,
    pub total_count: usize,
}

#[derive(Serialize)]
pub struct ModelSyncReportResponse {
    pub model_name: String,
    pub provider_name: String,
    pub discrepancies: Vec<ModelDiscrepancyResponse>,
    pub is_synced: bool,
}

#[derive(Serialize)]
pub struct ModelDiscrepancyResponse {
    pub field: String,
    pub database_value: Option<String>,
    pub api_value: Option<String>,
    pub severity: String,
}

#[derive(Deserialize)]
pub struct ModelSyncRequest {
    pub models: HashMap<String, DbModelInfo>,
}

#[derive(Serialize)]
pub struct ProviderModelItem {
    pub id: String,
    pub created: u32,
    pub owned_by: String,
}

#[derive(Serialize)]
pub struct ProviderModelsResponse {
    pub provider: String,
    pub models: Vec<ProviderModelItem>,
    pub total_count: usize,
}

pub async fn list_models(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
) -> Json<ModelsListResponse> {
    let user = &authenticated_user.user;

    let providers = state.config.router.get_providers().await;
    let routing_configs = state.config.db.list_routing_configs().await.unwrap_or_default();
    let mut all_models = Vec::new();

    let payments_enabled = state.payments_state.is_some();

    // Load user's model permissions for filtering
    let permissions = state.db.list_user_model_permissions(user.id).await.unwrap_or_default();

    // Check if user has a wildcard deny (*) — if so, deny everything
    let wildcard_deny = permissions.iter().any(|p| p.model == "*" && !p.allow);
    if wildcard_deny {
        return Json(ModelsListResponse {
            object: "list".to_string(),
            data: vec![],
        });
    }

    // Check if user has a wildcard allow (*) — if so, show everything
    let wildcard_allow = permissions.iter().any(|p| p.model == "*" && p.allow);

    // Helper to check if a model_id is allowed for this user
    let model_allowed = |model_id: &str| -> bool {
        if wildcard_allow {
            // Wildcard allow overrides everything, skip checks unless there's a specific deny
            if permissions.iter().any(|p| p.model == model_id && !p.allow) {
                return false;
            }
            return true;
        }

        // No wildcard: default-allow, but check for explicit deny or allow
        // If no permissions at all, allow everything
        if permissions.is_empty() {
            return true;
        }

        // Check for explicit deny first
        if permissions.iter().any(|p| p.model == model_id && !p.allow) {
            return false;
        }

        // Check for explicit allow
        if permissions.iter().any(|p| p.model == model_id && p.allow) {
            return true;
        }

        // If there are permissions defined but no rule matches this model,
        // default to denying (only explicitly allowed models are visible)
        false
    };

    // Add routing configs (routing engines) as models
    for rc in &routing_configs {
        if !model_allowed(&rc.name) {
            continue;
        }
        all_models.push(ModelEntry {
            id: rc.name.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: rc.name.clone(),
            pricing: None,
        });
    }

    // Add actual models from providers with provider slug prefix
    for provider in &providers {
        let provider_slug = provider.slug();

        match provider.list_models().await {
            Ok(models) => {
                for model in models {
                    let full_id = format!("{}/{}", provider_slug, model.id);

                    if !model_allowed(&full_id) {
                        continue;
                    }

                    // Resolve pricing for this model (even when payments disabled, show defaults)
                    let pricing = if payments_enabled {
                        if let Some(ref ps) = state.payments_state {
                            let p = ps.pricing_resolver.resolve(&model.id).await;
                            if !p.is_advertised {
                                continue; // skip unadvertised models
                            }
                            Some(ModelPricing {
                                prompt: p.price_per_1m_input_sats,
                                completion: p.price_per_1m_output_sats,
                                request: p.price_per_request_sats,
                                unit: "1M tokens".to_string(),
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    all_models.push(ModelEntry {
                        id: full_id,
                        object: model.object,
                        created: model.created as i64,
                        owned_by: model.owned_by,
                        pricing,
                    });
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Truncate error message to avoid logging huge JSON responses
                let short_error = if error_msg.len() > 200 {
                    format!("{}... (truncated)", &error_msg[..200])
                } else {
                    error_msg
                };
                tracing::warn!(
                    provider = provider.name(),
                    error = %short_error,
                    "Failed to list models from provider"
                );
            }
        }
    }

    Json(ModelsListResponse {
        object: "list".to_string(),
        data: all_models,
    })
}

#[axum::debug_handler]
pub async fn detect_model_discrepancies(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<ModelSyncRequest>,
) -> Json<Vec<ModelSyncReportResponse>> {
    let providers = state.config.router.get_providers().await;
    let detector = ModelInfoDetector::new(providers);

    let reports = detector.detect_discrepancies(&request.models).await;

    let response: Vec<ModelSyncReportResponse> = reports
        .into_iter()
        .map(|report| {
            let discrepancies = report.discrepancies
                .into_iter()
                .map(|d| ModelDiscrepancyResponse {
                    field: d.field,
                    database_value: d.database_value,
                    api_value: d.api_value,
                    severity: match d.severity {
                        crate::router::DiscrepancySeverity::Info => "info".to_string(),
                        crate::router::DiscrepancySeverity::Warning => "warning".to_string(),
                        crate::router::DiscrepancySeverity::Error => "error".to_string(),
                    },
                })
                .collect();

            ModelSyncReportResponse {
                model_name: report.model_name,
                provider_name: report.provider_name,
                discrepancies,
                is_synced: report.is_synced,
            }
        })
        .collect();

    Json(response)
}

#[axum::debug_handler]
pub async fn sync_provider_models(
    Path(provider_slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<SyncModelsResponse>, (axum::http::StatusCode, String)> {
    let providers = state.config.router.get_providers().await;
    let provider = providers
        .iter()
        .find(|p| p.slug() == provider_slug)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", provider_slug)))?;

    match provider.list_models().await {
        Ok(models) => {
            let mut model_details = Vec::new();

            for model in &models {
                match provider.get_runtime_info(&model.id).await {
                    Ok(Some(info)) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "object": model.object,
                            "created": model.created,
                            "owned_by": model.owned_by,
                            "context_length": info.context_length(),
                            "quantization": info.quantization(),
                            "parameter_size": info.parameter_size(),
                            "max_output_tokens": info.max_output_tokens,
                            "additional_fields": info.additional_fields,
                        }));
                    }
                    Ok(None) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "object": model.object,
                            "created": model.created,
                            "owned_by": model.owned_by,
                            "runtime_info": null,
                        }));
                    }
                    Err(e) => {
                        model_details.push(serde_json::json!({
                            "model_id": model.id,
                            "error": e.to_string(),
                        }));
                    }
                }
            }

            Ok(Json(SyncModelsResponse {
                provider: provider_slug,
                models: model_details,
                total_count: models.len(),
            }))
        }
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[axum::debug_handler]
pub async fn list_provider_models(
    Path(provider_slug): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<ProviderModelsResponse>, (axum::http::StatusCode, String)> {
    let providers = state.config.router.get_providers().await;
    let provider = providers
        .iter()
        .find(|p| p.slug() == provider_slug)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, format!("Provider '{}' not found", provider_slug)))?;

    match provider.list_models().await {
        Ok(models) => {
            let model_items: Vec<ProviderModelItem> = models
                .into_iter()
                .map(|m| ProviderModelItem {
                    id: m.id,
                    created: m.created,
                    owned_by: m.owned_by,
                })
                .collect();

            let total = model_items.len();
            Ok(Json(ProviderModelsResponse {
                provider: provider_slug,
                models: model_items,
                total_count: total,
            }))
        }
        Err(e) => Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}
