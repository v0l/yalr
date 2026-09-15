use axum::{
    extract::{Extension, Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use bytes::Bytes;
use std::sync::Arc;

use crate::auth::admin::AuthenticatedUser;
use crate::db::ModelAccess;
use crate::providers::audio::{SpeechRequest, TranscriptionRequest};
use crate::state::AppState;

type ApiError = (StatusCode, String);

fn error_body(message: &str, kind: &str) -> String {
    serde_json::json!({ "error": { "message": message, "type": kind } }).to_string()
}

async fn check_model_access(
    state: &AppState,
    user_id: i64,
    model: &str,
) -> Result<(), ApiError> {
    let access = state.db.check_model_access(user_id, model).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to check model access");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body(&format!("Failed to check model access: {e}"), "internal_error"),
        )
    })?;
    if access == ModelAccess::Deny {
        return Err((
            StatusCode::FORBIDDEN,
            error_body(
                &format!("User does not have access to model '{model}'"),
                "model_access_denied",
            ),
        ));
    }
    Ok(())
}

fn metrics_user(authenticated: &AuthenticatedUser) -> crate::metrics::MetricsUser {
    crate::metrics::MetricsUser {
        id: Some(authenticated.user.id),
        name: authenticated.user.username.clone(),
        api_key_id: authenticated.api_key.as_ref().map(|k| k.id),
        api_key_name: authenticated.api_key.as_ref().map(|k| k.name.clone()),
    }
}

fn router_error(e: crate::router::RouterError) -> ApiError {
    use crate::router::RouterError;
    use crate::ProviderError;

    match &e {
        RouterError::NoAvailableProvider => (
            StatusCode::NOT_FOUND,
            error_body(&e.to_string(), "router_error"),
        ),
        RouterError::ProviderError(ProviderError::Unsupported(_)) => (
            StatusCode::NOT_FOUND,
            error_body(
                "No configured provider can serve audio for this model",
                "model_not_supported",
            ),
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            error_body(&e.to_string(), "router_error"),
        ),
    }
}

pub async fn transcriptions(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    audio_stt(state, user, multipart, false).await
}

pub async fn translations(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    audio_stt(state, user, multipart, true).await
}

async fn audio_stt(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    multipart: Multipart,
    translate: bool,
) -> Result<Response, ApiError> {
    let request = parse_transcription_form(multipart, translate).await?;

    check_model_access(&state, authenticated.user.id, &request.model).await?;

    tracing::info!(
        model = %request.model,
        translate,
        bytes = request.file.len(),
        "Received audio transcription request"
    );

    let response = state
        .config
        .router
        .transcriptions(&request, Some(metrics_user(&authenticated)))
        .await
        .map_err(router_error)?;

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
    Json(request): Json<SpeechRequest>,
) -> Result<Response, ApiError> {
    if request.input.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            error_body("`input` must not be empty", "invalid_request_error"),
        ));
    }

    check_model_access(&state, authenticated.user.id, &request.model).await?;

    tracing::info!(
        model = %request.model,
        chars = request.input.chars().count(),
        "Received audio speech request"
    );

    let response = state
        .config
        .router
        .speech(&request, Some(metrics_user(&authenticated)))
        .await
        .map_err(router_error)?;

    let body = axum::body::Body::from_stream(response.stream);
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
        let db = Database::new("sqlite::memory:").await.unwrap();
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
