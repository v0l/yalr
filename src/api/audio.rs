use axum::{
    extract::{ConnectInfo, Extension, Multipart, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::api::metered::{response_body, usage_in_body, Metered, MultipartBody, RequestInfo, UploadedFile};

use crate::auth::admin::AuthenticatedUser;
use crate::api::modalities::require_modalities;
use crate::api::shared::{check_model_access, error_body, metrics_user, router_error, HandlerError as ApiError};
use crate::providers::audio::{SpeechRequest, TranscriptionRequest};
use crate::router::Modality;
use crate::state::AppState;

pub async fn transcriptions(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    audio_stt(state, user, peer, headers, multipart, false).await
}

pub async fn translations(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    audio_stt(state, user, peer, headers, multipart, true).await
}

fn transcription_log_body(request: &TranscriptionRequest) -> Vec<u8> {
    let temperature = request.temperature.map(|t| t.to_string());
    let mut fields = vec![("model", request.model.as_str())];
    for (name, value) in [
        ("language", request.language.as_deref()),
        ("prompt", request.prompt.as_deref()),
        ("response_format", request.response_format.as_deref()),
        ("temperature", temperature.as_deref()),
    ] {
        if let Some(value) = value {
            fields.push((name, value));
        }
    }
    for granularity in &request.timestamp_granularities {
        fields.push(("timestamp_granularities[]", granularity));
    }
    let file = UploadedFile::new("file", &request.file_name, request.content_type.as_deref(), &request.file);
    serde_json::to_vec(&MultipartBody { fields, files: vec![file] }).unwrap_or_default()
}

async fn audio_stt(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    multipart: Multipart,
    translate: bool,
) -> Result<Response, ApiError> {
    let request = parse_transcription_form(multipart, translate).await?;
    let endpoint = if translate { "/v1/audio/translations" } else { "/v1/audio/transcriptions" };
    let info = RequestInfo { endpoint, model: &request.model, stream: false, headers: &headers, peer: peer.map(|p| p.0) };
    let mut metered = Metered::begin(&state, &authenticated, info, || transcription_log_body(&request));

    if let Err(e) = check_model_access(&state, authenticated.user.id, &request.model).await {
        return Err(metered.reject(e));
    }
    if let Err(e) = require_modalities(&state, &request.model, &[Modality::Audio], &[]).await {
        return Err(metered.reject(e));
    }
    if let Err(e) = metered.admit().await {
        return Err(metered.reject(e));
    }

    tracing::info!(
        model = %request.model,
        translate,
        bytes = request.file.len(),
        "Received audio transcription request"
    );

    let response = match state
        .config
        .router
        .transcriptions_traced(&request, Some(metrics_user(&authenticated)), Some(metered.trace()))
        .await
    {
        Ok(response) => response,
        Err(e) => {
            metered.fail(&e.to_string());
            return Err(router_error(e, "audio"));
        }
    };
    let usage = usage_in_body(&response.content_type, &response.body);
    metered.succeed(response_body(&response.content_type, &response.body), usage).await;

    Ok((
        [(header::CONTENT_TYPE, response.content_type)],
        response.body,
    )
        .into_response())
}

#[derive(serde::Deserialize)]
pub struct VoicesQuery {
    pub model: String,
}

#[derive(serde::Serialize)]
pub struct VoicesResponse {
    pub model: String,
    pub voices: Vec<String>,
}

/// Voice names the backends behind `model` accept. Empty when the upstream
/// does not publish a list, which is not the same as "takes no voice".
pub async fn voices(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    axum::extract::Query(query): axum::extract::Query<VoicesQuery>,
) -> Result<Json<VoicesResponse>, ApiError> {
    check_model_access(&state, authenticated.user.id, &query.model).await?;
    let voices = state.config.router.voices(&query.model).await;
    Ok(Json(VoicesResponse { model: query.model, voices }))
}

pub async fn speech(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: SpeechRequest = serde_json::from_slice(&body).map_err(|e| {
        (StatusCode::UNPROCESSABLE_ENTITY, error_body(&format!("Invalid request body: {e}"), "invalid_request_error"))
    })?;
    if request.input.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            error_body("`input` must not be empty", "invalid_request_error"),
        ));
    }

    let stream = request.extra.get("stream_format").and_then(|v| v.as_str()) == Some("sse");
    let info = RequestInfo { endpoint: "/v1/audio/speech", model: &request.model, stream, headers: &headers, peer: peer.map(|p| p.0) };
    let mut metered = Metered::begin(&state, &authenticated, info, || body.to_vec());

    if let Err(e) = check_model_access(&state, authenticated.user.id, &request.model).await {
        return Err(metered.reject(e));
    }
    if let Err(e) = require_modalities(&state, &request.model, &[], &[Modality::Audio]).await {
        return Err(metered.reject(e));
    }
    if let Err(e) = metered.admit().await {
        return Err(metered.reject(e));
    }

    tracing::info!(
        model = %request.model,
        chars = request.input.chars().count(),
        "Received audio speech request"
    );

    let response = match state
        .config
        .router
        .speech_traced(&request, Some(metrics_user(&authenticated)), Some(metered.trace()))
        .await
    {
        Ok(response) => response,
        Err(e) => {
            metered.fail(&e.to_string());
            return Err(router_error(e, "audio"));
        }
    };

    let tapped = metered.tap(response.content_type.clone(), response.stream);
    let body = axum::body::Body::from_stream(tapped);
    Ok(([(header::CONTENT_TYPE, response.content_type)], body).into_response())
}

/// Parse an OpenAI-shaped `multipart/form-data` transcription request.
async fn parse_transcription_form(
    mut multipart: Multipart,
    translate: bool,
) -> Result<TranscriptionRequest, ApiError> {
    let bad_request = |message: String| (StatusCode::BAD_REQUEST, error_body(&message, "invalid_request_error"));

    let mut model = None;
    let mut file: Option<Bytes> = None;
    let mut file_name = "audio".to_string();
    let mut content_type = None;
    let mut language = None;
    let mut prompt = None;
    let mut response_format = None;
    let mut temperature = None;
    let mut timestamp_granularities = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(format!("Malformed multipart body: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" => {
                if let Some(n) = field.file_name() {
                    file_name = n.to_string();
                }
                content_type = field.content_type().map(str::to_string);
                file = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| bad_request(format!("Failed to read audio file: {e}")))?,
                );
            }
            _ => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| bad_request(format!("Failed to read field `{name}`: {e}")))?;
                match name.as_str() {
                    "model" => model = Some(value),
                    "language" => language = Some(value),
                    "prompt" => prompt = Some(value),
                    "response_format" => response_format = Some(value),
                    "temperature" => {
                        temperature = Some(value.parse::<f32>().map_err(|_| {
                            bad_request(format!("`temperature` is not a number: {value}"))
                        })?)
                    }
                    "timestamp_granularities" | "timestamp_granularities[]" => {
                        timestamp_granularities.push(value)
                    }
                    _ => {}
                }
            }
        }
    }

    let model = model.ok_or_else(|| bad_request("Missing required field `model`".into()))?;
    let file = file.ok_or_else(|| bad_request("Missing required field `file`".into()))?;
    if file.is_empty() {
        return Err(bad_request("Uploaded audio file is empty".into()));
    }

    Ok(TranscriptionRequest {
        model,
        file_name,
        file,
        content_type,
        language,
        prompt,
        response_format,
        temperature,
        timestamp_granularities,
        translate,
    })
}

#[cfg(test)]
mod tests {
    use crate::api::server::create_test_app;
    use crate::auth::admin::SessionStore;
    use crate::db::{Database, NewUser, UserType};
    use crate::metrics::MetricsStore;
    use crate::providers::OpenAiProvider;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::util::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const BOUNDARY: &str = "----yalrtestboundary";

    async fn setup(upstream: &str) -> (Arc<AppState>, String) {
        let db = Database::test().await;
        let metrics_store = MetricsStore::new(1000);

        let router = Arc::new(crate::router::engine::Router::new(
            metrics_store.clone(),
            Arc::new(db.clone()),
        ));
        let provider = Arc::new(OpenAiProvider::new("whisper", Some("whisper"), upstream, Some("k")));
        router.add_provider(provider.clone()).await;
        router.register_route("whisper-1", vec![provider.clone()]).await;
        router.register_route("tts-1", vec![provider]).await;

        let app_config = crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router,
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
            discovery_config: None,
            admin_ui_path: "/app/admin/dist".to_string(),
            host: "0.0.0.0".to_string(),
            port: 3000,
        };

        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: app_config,
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.clone().into(),
            session_store: session_store.clone(),
            db: Arc::new(db),
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
        });

        state
            .db
            .create_user(NewUser {
                username: Some("admin"),
                password_hash: Some("x"),
                external_id: None,
                user_type: UserType::Internal,
                is_admin: true,
            })
            .await
            .unwrap();
        let token = session_store.create("admin", true, 86400).await;

        (state, token)
    }

    fn multipart_body(model: &str, audio: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(model.as_bytes());
        body.extend_from_slice(format!("\r\n--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"clip.wav\"\r\n\
              Content-Type: audio/wav\r\n\r\n",
        );
        body.extend_from_slice(audio);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    #[tokio::test]
    async fn transcription_round_trip_over_http() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(r#"{"text":"round trip"}"#, "application/json"),
            )
            .mount(&upstream)
            .await;

        let (state, token) = setup(&upstream.uri()).await;
        let app = create_test_app(state).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audio/transcriptions")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
                    .body(Body::from(multipart_body("whisper-1", b"RIFFDATA")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&body[..], br#"{"text":"round trip"}"#);
    }

    #[tokio::test]
    async fn speech_round_trip_over_http() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![9u8, 8, 7], "audio/mpeg"))
            .mount(&upstream)
            .await;

        let (state, token) = setup(&upstream.uri()).await;
        let app = create_test_app(state).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audio/speech")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"tts-1","input":"hi","voice":"alloy"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers().get("content-type").unwrap().to_str().unwrap(),
            "audio/mpeg"
        );
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&body[..], &[9u8, 8, 7]);
    }

    #[tokio::test]
    async fn audio_routes_require_auth() {
        let upstream = MockServer::start().await;
        let (state, _) = setup(&upstream.uri()).await;
        let app = create_test_app(state).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audio/speech")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"tts-1","input":"hi"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn empty_input_is_rejected_before_routing() {
        let upstream = MockServer::start().await;
        let (state, token) = setup(&upstream.uri()).await;
        let app = create_test_app(state).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audio/speech")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"tts-1","input":""}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
    }

    #[tokio::test]
    async fn missing_model_field_is_a_bad_request() {
        let upstream = MockServer::start().await;
        let (state, token) = setup(&upstream.uri()).await;
        let app = create_test_app(state).await;

        let mut body = Vec::new();
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"clip.wav\"\r\n\r\nX\r\n",
        );
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audio/transcriptions")
                    .method("POST")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", format!("multipart/form-data; boundary={BOUNDARY}"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
    }
}
