use crate::providers::Provider;
use crate::router::{DbModelInfo, ModelInfoDetector};
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    Extension,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Context window in tokens. Only set when configured in `model_pricing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<i32>,
    /// What the upstream accepts as input, in the OpenAI/LM Studio
    /// `architecture.input_modalities` shape. Omitted when no backend can
    /// report it, so clients fall back to their own default instead of being
    /// told a confident "text only" we cannot actually verify.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<ModelArchitecture>,
}

#[derive(Serialize)]
pub struct ModelArchitecture {
    pub input_modalities: Vec<String>,
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

/// Deadline for a single capability probe. `get_runtime_info` has no internal
/// timeout, and the models list must not inherit a hung upstream's latency.
const MODALITY_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Input modalities for a single provider model, from the provider's own
/// capabilities endpoint. `None` when the provider cannot report them, so
/// callers can tell "text only" apart from "unknown".
async fn provider_input_modalities(
    state: &AppState,
    provider: &Arc<dyn Provider>,
    model_id: &str,
) -> Option<ModelArchitecture> {
    if !provider.reports_modalities() {
        return None;
    }

    // Skip a provider the health checker has already written off: its probe
    // would burn the full timeout on every listing request and still fail.
    if !state.metrics_store.is_provider_available(provider.name()).await {
        return None;
    }

    let key = format!("{}/{}", provider.slug(), model_id);
    let input_modalities = match state.modality_cache.get(&key).await {
        Some(cached) => cached,
        None => {
            let probe = tokio::time::timeout(
                MODALITY_PROBE_TIMEOUT,
                provider.get_runtime_info(model_id),
            )
            .await;
            let modalities: Vec<String> = match probe {
                Ok(Ok(Some(info))) => info
                    .modalities
                    .iter()
                    .map(|m| m.as_str().to_string())
                    .collect(),
                // Do not cache a failed or timed-out probe as text-only: a
                // provider that is briefly unreachable must not pin an
                // unverifiable answer for the whole TTL. The next request retries.
                _ => return None,
            };
            state.modality_cache.put(&key, modalities.clone()).await;
            modalities
        }
    };

    Some(ModelArchitecture { input_modalities })
}

/// Input modalities for a routing engine: the union across its backends.
///
/// A routing engine is a pool, not one model. The union answers "can a request
/// through this model carry an image" - at least one backend accepts it, and
/// the engine fails over on the rejection from a text-only backend. Backends
/// that cannot report modalities are skipped rather than treated as text-only,
/// so a pool is not downgraded just because one member has no capabilities
/// endpoint. `None` when no backend can report, which preserves the client's
/// own default.
async fn routing_input_modalities(state: &AppState, model: &str) -> Option<ModelArchitecture> {
    let backends = state.config.router.candidate_backends(model).await;
    let mut input_modalities: Vec<String> = Vec::new();
    let mut reported = false;

    for (provider, resolved_model) in backends {
        let Some(architecture) =
            provider_input_modalities(state, &provider, &resolved_model).await
        else {
            continue;
        };
        reported = true;
        for modality in architecture.input_modalities {
            if !input_modalities.contains(&modality) {
                input_modalities.push(modality);
            }
        }
    }

    reported.then_some(ModelArchitecture { input_modalities })
}

pub async fn list_models(
    State(state): State<std::sync::Arc<AppState>>,
    Extension(authenticated_user): Extension<crate::auth::admin::AuthenticatedUser>,
) -> Json<ModelsListResponse> {
    let user = &authenticated_user.user;

    let providers = state.config.router.get_providers().await;
    let routing_configs = state.config.db.list_routing_configs().await.unwrap_or_default();
    let mut all_models = Vec::new();

    // Context/output limits come from explicit `model_pricing` rows only; an
    // unset value is omitted rather than defaulted, so clients can't be told a
    // placeholder 8k window for a model that actually has more.
    let limits: HashMap<String, (Option<i32>, Option<i32>)> = state
        .db
        .list_model_pricings()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|mp| (mp.model_name, (mp.context_window, mp.max_output_tokens)))
        .collect();

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
        let (context_length, max_output_tokens) =
            limits.get(&rc.name).copied().unwrap_or((None, None));
        all_models.push(ModelEntry {
            id: rc.name.clone(),
            object: "model".to_string(),
            created: 0,
            owned_by: rc.name.clone(),
            pricing: None,
            context_length,
            max_output_tokens,
            architecture: routing_input_modalities(&state, &rc.name).await,
        });
    }

    // Fan out to every provider at once with a hard deadline: this endpoint used
    // to await providers one at a time with no request timeout, so a single
    // upstream that accepted the connection and then stalled hung the whole list.
    // Providers the health checker has already marked down are not dialled at
    // all; their last good listing is served instead.
    let state_ref = &state;
    let listings = futures::future::join_all(providers.iter().map(|provider| async move {
        let provider_slug = provider.slug();

        if !state_ref
            .metrics_store
            .is_provider_available(provider.name())
            .await
        {
            let cached = state_ref.model_cache.get(&provider_slug).await;
            tracing::debug!(
                provider = provider.name(),
                cached = cached.as_ref().map_or(0, |m| m.len()),
                "Provider unavailable, serving cached model list"
            );
            return (provider, cached.unwrap_or_default());
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            provider.list_models(),
        )
        .await;

        let models = match result {
            Ok(Ok(models)) => {
                state_ref.model_cache.put(&provider_slug, models.clone()).await;
                models
            }
            Ok(Err(e)) => {
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
                state_ref.model_cache.get(&provider_slug).await.unwrap_or_default()
            }
            Err(_) => {
                tracing::warn!(
                    provider = provider.name(),
                    "Timed out listing models from provider"
                );
                state_ref.model_cache.get(&provider_slug).await.unwrap_or_default()
            }
        };

        (provider, models)
    }))
    .await;

    // Add actual models from providers with provider slug prefix
    for (provider, models) in listings {
        let provider_slug = provider.slug();

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

            let (context_length, max_output_tokens) = limits
                .get(&model.id)
                .or_else(|| limits.get(&full_id))
                .copied()
                .unwrap_or((None, None));

            all_models.push(ModelEntry {
                id: full_id,
                object: model.object,
                created: model.created as i64,
                owned_by: model.owned_by,
                pricing,
                context_length,
                max_output_tokens,
                architecture: provider_input_modalities(&state, provider, &model.id).await,
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::admin::SessionStore;
    use crate::config::AppConfig;
    use crate::db::Database;
    use crate::metrics::MetricsStore;
    use crate::providers::{LlamaCppProvider, OpenAiProvider};
    use crate::router::engine::Router;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Minimal llama.cpp `/props` stand-in. Returns `vision` for the whole
    /// process, because that is exactly the shape `LlamaCppProvider` arrives at:
    /// one server, one loaded model, one set of modalities.
    async fn start_props_server(vision: bool) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_task = hits.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                hits_for_task.fetch_add(1, Ordering::SeqCst);
                let body = format!(
                    r#"{{"model_alias":"glm-5.3-flash","total_slots":2,"modalities":{{"vision":{},"audio":false}},"default_generation_settings":{{"n_ctx":262144}}}}"#,
                    vision
                );
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        (format!("http://127.0.0.1:{}/v1", addr.port()), hits)
    }

    async fn state_with(provider: Arc<dyn Provider>, model: &str) -> Arc<AppState> {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let metrics_store = MetricsStore::new(100);
        let router = Arc::new(Router::new(metrics_store.clone(), db.clone()));
        router.add_provider(provider.clone()).await;
        router.register_route(model, vec![provider]).await;

        Arc::new(AppState {
            config: AppConfig {
                db: db.clone(),
                router,
                auth_config: Default::default(),
                payments_config: None,
                admin_ui_path: String::new(),
                host: "127.0.0.1".to_string(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.into(),
            session_store: Arc::new(SessionStore::new()),
            db,
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
        })
    }

    fn vision_provider(base_url: &str) -> Arc<dyn Provider> {
        Arc::new(LlamaCppProvider::new("LocalVision", Some("local"), base_url, None).unwrap())
    }

    #[tokio::test]
    async fn routing_engine_advertises_vision_from_its_backend() {
        let (base, _) = start_props_server(true).await;
        let state = state_with(vision_provider(&base), "code-high").await;

        let architecture = routing_input_modalities(&state, "code-high").await.unwrap();
        assert_eq!(architecture.input_modalities, vec!["text", "image"]);
    }

    #[tokio::test]
    async fn text_only_backend_advertises_text_only() {
        let (base, _) = start_props_server(false).await;
        let state = state_with(vision_provider(&base), "code").await;

        let architecture = routing_input_modalities(&state, "code").await.unwrap();
        assert_eq!(architecture.input_modalities, vec!["text"]);
    }

    #[tokio::test]
    async fn modality_probe_is_cached() {
        let (base, hits) = start_props_server(true).await;
        let state = state_with(vision_provider(&base), "code-high").await;

        for _ in 0..3 {
            assert!(routing_input_modalities(&state, "code-high").await.is_some());
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "one /props probe, then cached");
    }

    #[tokio::test]
    async fn provider_without_modality_support_yields_none() {
        // Every non-llama.cpp provider hardcodes `Text`; reporting that as a
        // fact would override the client's own default with a guess.
        let provider: Arc<dyn Provider> = Arc::new(OpenAiProvider::new(
            "OpenAI",
            Some("openai"),
            "http://127.0.0.1:1/v1",
            Some("key"),
        ));
        assert!(!provider.reports_modalities());
        let state = state_with(provider, "gpt").await;

        let backends = state.config.router.candidate_backends("gpt").await;
        assert_eq!(backends.len(), 1);
        assert!(provider_input_modalities(&state, &backends[0].0, "gpt").await.is_none());
        assert!(routing_input_modalities(&state, "gpt").await.is_none());
    }

    #[test]
    fn model_entry_omits_architecture_when_unknown() {
        let entry = ModelEntry {
            id: "code-high".to_string(),
            object: "model".to_string(),
            created: 0,
            owned_by: "code-high".to_string(),
            pricing: None,
            context_length: Some(1048576),
            max_output_tokens: Some(65536),
            architecture: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(
            json.get("architecture").is_none(),
            "unknown capabilities must be omitted, not reported as text-only"
        );
    }

    #[test]
    fn model_entry_serializes_input_modalities_lowercase() {
        let entry = ModelEntry {
            id: "llamacpp/glm-5.3-flash".to_string(),
            object: "model".to_string(),
            created: 0,
            owned_by: "llamacpp".to_string(),
            pricing: None,
            context_length: None,
            max_output_tokens: None,
            architecture: Some(ModelArchitecture {
                input_modalities: vec!["text".to_string(), "image".to_string()],
            }),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            json["architecture"]["input_modalities"],
            serde_json::json!(["text", "image"])
        );
    }
}
