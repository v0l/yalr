use std::sync::Arc;

use axum::http::StatusCode;
use serde::Serialize;

use crate::api::shared::{error_body, HandlerError};
use crate::db::{DeclaredModalities, RoutingConfig};
use crate::providers::{ChatRequest, Provider};
use crate::router::Modality;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ModelArchitecture {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
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
) -> Option<Vec<String>> {
    if let Some(listed) = provider.listed_modalities(model_id).await {
        return Some(listed.input_modalities);
    }
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

    Some(input_modalities)
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
async fn probed_input_modalities(state: &AppState, model: &str) -> Option<Vec<String>> {
    let backends = state.config.router.candidate_backends(model).await;
    let mut input_modalities: Vec<String> = Vec::new();
    let mut reported = false;

    for (provider, resolved_model) in backends {
        let Some(modalities) =
            provider_input_modalities(state, &provider, &resolved_model).await
        else {
            continue;
        };
        reported = true;
        for modality in modalities {
            if !input_modalities.contains(&modality) {
                input_modalities.push(modality);
            }
        }
    }

    reported.then_some(input_modalities)
}

pub async fn provider_architecture(
    state: &AppState,
    provider: &Arc<dyn Provider>,
    model_id: &str,
) -> Option<ModelArchitecture> {
    let output = provider
        .listed_modalities(model_id)
        .await
        .map(|listed| listed.output_modalities);
    let input = provider_input_modalities(state, provider, model_id).await;
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(ModelArchitecture {
        input_modalities: input.unwrap_or_default(),
        output_modalities: output.unwrap_or_default(),
    })
}

async fn listed_output_modalities(state: &AppState, model: &str) -> Option<Vec<String>> {
    let mut output: Vec<String> = Vec::new();
    let mut listed = false;
    for (provider, resolved_model) in state.config.router.candidate_backends(model).await {
        let Some(modalities) = provider.listed_modalities(&resolved_model).await else {
            continue;
        };
        listed = true;
        for modality in modalities.output_modalities {
            if !output.contains(&modality) {
                output.push(modality);
            }
        }
    }
    listed.then_some(output)
}

pub async fn routing_architecture(state: &AppState, rc: &RoutingConfig) -> Option<ModelArchitecture> {
    let declared = rc.declared_modalities();
    let input = match DeclaredModalities::names(&declared.input) {
        Some(names) => Some(names),
        None => probed_input_modalities(state, &rc.name).await,
    };
    let output = match DeclaredModalities::names(&declared.output) {
        Some(names) => Some(names),
        None => listed_output_modalities(state, &rc.name).await,
    };
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(ModelArchitecture {
        input_modalities: input.unwrap_or_default(),
        output_modalities: output.unwrap_or_default(),
    })
}

pub async fn require_modalities(
    state: &AppState,
    model: &str,
    input: &[Modality],
    output: &[Modality],
) -> Result<(), HandlerError> {
    if input.is_empty() && output.is_empty() {
        return Ok(());
    }
    let rc = match state.db.get_routing_config_by_name(model).await {
        Ok(Some(rc)) => rc,
        Ok(None) => return Ok(()),
        Err(e) => {
            tracing::warn!(model, error = %e, "Failed to load routing config for modality check");
            return Ok(());
        }
    };
    match rc.declared_modalities().rejection(input, output) {
        Some(reason) => Err((
            StatusCode::BAD_REQUEST,
            error_body(&format!("Model '{model}' {reason}"), "model_not_supported"),
        )),
        None => Ok(()),
    }
}

pub fn chat_modalities(request: &ChatRequest) -> (Vec<Modality>, Vec<Modality>) {
    let input = request.input_modalities();
    let output = request
        .output_modalities()
        .iter()
        .filter_map(|m| Modality::parse(m))
        .filter(|m| *m != Modality::Text)
        .collect();
    (input, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::NewRoutingConfig;
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

        let architecture = probed_input_modalities(&state, "code-high").await.unwrap();
        assert_eq!(architecture, vec!["text", "image"]);
    }

    #[tokio::test]
    async fn text_only_backend_advertises_text_only() {
        let (base, _) = start_props_server(false).await;
        let state = state_with(vision_provider(&base), "code").await;

        let architecture = probed_input_modalities(&state, "code").await.unwrap();
        assert_eq!(architecture, vec!["text"]);
    }

    #[tokio::test]
    async fn modality_probe_is_cached() {
        let (base, hits) = start_props_server(true).await;
        let state = state_with(vision_provider(&base), "code-high").await;

        for _ in 0..3 {
            assert!(probed_input_modalities(&state, "code-high").await.is_some());
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
        assert!(probed_input_modalities(&state, "gpt").await.is_none());
    }

    fn openai(model_state_url: &str) -> Arc<dyn Provider> {
        Arc::new(OpenAiProvider::new("OpenAI", Some("openai"), model_state_url, Some("key")))
    }

    async fn declare(state: &AppState, name: &str, declared: DeclaredModalities) -> RoutingConfig {
        let rc = state
            .db
            .create_routing_config(NewRoutingConfig {
                name: name.into(),
                strategy: "round_robin".into(),
                health_check_enabled: false,
                health_check_interval_seconds: 30,
                health_check_timeout_seconds: 5,
            })
            .await
            .unwrap();
        state.db.set_routing_config_modalities(rc.id, &declared).await.unwrap();
        state.db.get_routing_config(rc.id).await.unwrap().unwrap()
    }

    fn painter() -> DeclaredModalities {
        DeclaredModalities {
            input: Some(vec![Modality::Text]),
            output: Some(vec![Modality::Image]),
        }
    }

    #[tokio::test]
    async fn declared_modalities_are_published_without_a_probe() {
        let state = state_with(openai("http://127.0.0.1:1/v1"), "painter").await;
        let rc = declare(&state, "painter", painter()).await;

        let architecture = routing_architecture(&state, &rc).await.unwrap();
        assert_eq!(architecture.input_modalities, vec!["text"]);
        assert_eq!(architecture.output_modalities, vec!["image"]);
    }

    #[tokio::test]
    async fn declared_output_keeps_the_probed_input() {
        let (base, _) = start_props_server(true).await;
        let state = state_with(vision_provider(&base), "code-high").await;
        let rc = declare(
            &state,
            "code-high",
            DeclaredModalities {
                input: None,
                output: Some(vec![Modality::Text]),
            },
        )
        .await;

        let architecture = routing_architecture(&state, &rc).await.unwrap();
        assert_eq!(architecture.input_modalities, vec!["text", "image"]);
        assert_eq!(architecture.output_modalities, vec!["text"]);
    }

    #[tokio::test]
    async fn undeclared_config_with_no_probe_publishes_nothing() {
        let state = state_with(openai("http://127.0.0.1:1/v1"), "gpt").await;
        let rc = declare(&state, "gpt", DeclaredModalities::default()).await;
        assert!(routing_architecture(&state, &rc).await.is_none());
    }

    #[tokio::test]
    async fn gate_rejects_an_undeclared_modality() {
        let state = state_with(openai("http://127.0.0.1:1/v1"), "painter").await;
        declare(&state, "painter", painter()).await;

        assert!(require_modalities(&state, "painter", &[], &[Modality::Image]).await.is_ok());
        let (status, body) = require_modalities(&state, "painter", &[Modality::Audio], &[])
            .await
            .unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("does not accept audio input"), "{body}");
    }

    #[tokio::test]
    async fn gate_passes_unknown_and_undeclared_models() {
        let state = state_with(openai("http://127.0.0.1:1/v1"), "gpt").await;
        declare(&state, "gpt", DeclaredModalities::default()).await;

        assert!(require_modalities(&state, "gpt", &[Modality::Image], &[Modality::Image]).await.is_ok());
        assert!(require_modalities(&state, "openai/gpt-image-1", &[], &[Modality::Image]).await.is_ok());
    }

    fn chat(body: &str) -> ChatRequest {
        serde_json::from_str(body).unwrap()
    }

    #[test]
    fn chat_detects_image_and_audio_parts() {
        let request = chat(
            r#"{"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA"}},{"type":"input_audio","input_audio":{"data":"AA","format":"wav"}}]}]}"#,
        );
        assert_eq!(chat_modalities(&request), (vec![Modality::Image, Modality::Audio], vec![]));
    }

    #[test]
    fn plain_text_chat_needs_nothing() {
        let request = chat(r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#);
        assert_eq!(chat_modalities(&request), (vec![], vec![]));
    }

    #[test]
    fn audio_output_is_detected_from_modalities() {
        let request = chat(
            r#"{"model":"m","modalities":["text","audio"],"messages":[{"role":"user","content":"hi"}]}"#,
        );
        assert_eq!(chat_modalities(&request).1, vec![Modality::Audio]);
    }

    #[test]
    fn image_output_is_detected_from_modalities() {
        let request = chat(
            r#"{"model":"m","modalities":["image","text"],"messages":[{"role":"user","content":"draw"}]}"#,
        );
        assert_eq!(chat_modalities(&request).1, vec![Modality::Image]);
    }

    #[tokio::test]
    async fn openrouter_listing_publishes_both_modalities() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"data":[{"id":"openai/gpt-5.4-image-2","architecture":{"input_modalities":["image","text","file"],"output_modalities":["image","text"]}},{"id":"plain/text"}]}"#,
                "application/json",
            ))
            .mount(&server)
            .await;

        let provider: Arc<dyn Provider> = Arc::new(crate::providers::OpenRouterProvider::new(
            "OpenRouter",
            Some("openrouter"),
            &server.uri(),
            Some("k"),
        ));
        provider.list_models().await.unwrap();
        let state = state_with(provider.clone(), "painter").await;

        let architecture = provider_architecture(&state, &provider, "openai/gpt-5.4-image-2")
            .await
            .unwrap();
        assert_eq!(architecture.input_modalities, vec!["image", "text", "file"]);
        assert_eq!(architecture.output_modalities, vec!["image", "text"]);
        assert!(provider_architecture(&state, &provider, "plain/text").await.is_none());

        let rc = declare(&state, "painter", DeclaredModalities::default()).await;
        let routed = routing_architecture(&state, &rc).await;
        assert!(routed.is_none(), "painter routes to no listed model");
    }
}
