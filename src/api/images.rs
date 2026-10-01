use axum::{
    extract::{Extension, Multipart, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::api::shared::{check_model_access, error_body, metrics_user, router_error, HandlerError};
use crate::auth::admin::AuthenticatedUser;
use crate::providers::image::{
    ImageFile, ImageGenerationRequest, ImageResponse, ImageUploadKind, ImageUploadRequest,
};
use crate::state::AppState;

fn bad_request(message: impl AsRef<str>) -> HandlerError {
    (
        StatusCode::BAD_REQUEST,
        error_body(message.as_ref(), "invalid_request_error"),
    )
}

fn stream_response(response: ImageResponse) -> Response {
    let body = axum::body::Body::from_stream(response.stream);
    ([(header::CONTENT_TYPE, response.content_type)], body).into_response()
}

pub async fn generations(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    Json(request): Json<ImageGenerationRequest>,
) -> Result<Response, HandlerError> {
    if request.prompt.trim().is_empty() {
        return Err(bad_request("`prompt` must not be empty"));
    }

    check_model_access(&state, authenticated.user.id, &request.model).await?;

    tracing::info!(model = %request.model, n = ?request.n, "Received image generation request");

    let response = state
        .config
        .router
        .image_generations(&request, Some(metrics_user(&authenticated)))
        .await
        .map_err(|e| router_error(e, "image generation"))?;

    Ok(stream_response(response))
}

pub async fn edits(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    multipart: Multipart,
) -> Result<Response, HandlerError> {
    upload(state, user, multipart, ImageUploadKind::Edit).await
}

pub async fn variations(
    state: State<Arc<AppState>>,
    user: Extension<AuthenticatedUser>,
    multipart: Multipart,
) -> Result<Response, HandlerError> {
    upload(state, user, multipart, ImageUploadKind::Variation).await
}

async fn upload(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    multipart: Multipart,
    kind: ImageUploadKind,
) -> Result<Response, HandlerError> {
    let request = parse_upload_form(multipart, kind).await?;

    check_model_access(&state, authenticated.user.id, &request.model).await?;

    tracing::info!(
        model = %request.model,
        op = request.operation(),
        files = request.files.len(),
        "Received image upload request"
    );

    let response = state
        .config
        .router
        .image_uploads(&request, Some(metrics_user(&authenticated)))
        .await
        .map_err(|e| router_error(e, request.operation()))?;

    Ok(stream_response(response))
}

async fn parse_upload_form(
    mut multipart: Multipart,
    kind: ImageUploadKind,
) -> Result<ImageUploadRequest, HandlerError> {
    let mut model = None;
    let mut files = Vec::new();
    let mut fields = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(format!("Malformed multipart body: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if let Some(file_name) = field.file_name().map(str::to_string) {
            let content_type = field.content_type().map(str::to_string);
            let data = field
                .bytes()
                .await
                .map_err(|e| bad_request(format!("Failed to read file `{name}`: {e}")))?;
            if data.is_empty() {
                return Err(bad_request(format!("Uploaded file `{name}` is empty")));
            }
            files.push(ImageFile {
                field: name,
                file_name,
                content_type,
                data,
            });
            continue;
        }

        let value = field
            .text()
            .await
            .map_err(|e| bad_request(format!("Failed to read field `{name}`: {e}")))?;
        if name == "model" {
            model = Some(value);
        } else {
            fields.push((name, value));
        }
    }

    let model = model.ok_or_else(|| bad_request("Missing required field `model`"))?;
    if !files.iter().any(|f| f.field.starts_with("image")) {
        return Err(bad_request("Missing required field `image`"));
    }

    let request = ImageUploadRequest {
        kind,
        model,
        files,
        fields,
    };
    if kind == ImageUploadKind::Edit
        && request.field("prompt").is_none_or(|p| p.trim().is_empty())
    {
        return Err(bad_request("Missing required field `prompt`"));
    }
    Ok(request)
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
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const BOUNDARY: &str = "----yalrimageboundary";
    const IMAGES: &str = r#"{"created":1,"data":[{"b64_json":"aGk="}]}"#;

    async fn app(upstream: &str) -> (axum::Router, String) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let metrics_store = MetricsStore::new(1000);
        let router = Arc::new(crate::router::engine::Router::new(
            metrics_store.clone(),
            Arc::new(db.clone()),
        ));
        let provider = Arc::new(OpenAiProvider::new("openai", Some("openai"), upstream, Some("k")));
        router.add_provider(provider.clone()).await;
        router.register_route("gpt-image-1", vec![provider]).await;

        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: crate::config::AppConfig {
                db: Arc::new(db.clone()),
                router,
                auth_config: crate::auth::nip98::AuthConfig::default(),
                payments_config: None,
                admin_ui_path: "/app/admin/dist".to_string(),
                host: "0.0.0.0".to_string(),
                port: 3000,
            },
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
        (create_test_app(state).await, token)
    }

    fn form(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, file_name, data) in parts {
            body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
            match file_name {
                Some(f) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{f}\"\r\nContent-Type: image/png\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        body
    }

    fn post(uri: &str, token: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .method("POST")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap()
    }

    fn multipart() -> String {
        format!("multipart/form-data; boundary={BOUNDARY}")
    }

    #[tokio::test]
    async fn generation_round_trip_over_http() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .and(body_string_contains(r#""model":"gpt-image-1""#))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&upstream)
            .await;
        let (app, token) = app(&upstream.uri()).await;

        let response = app
            .oneshot(post(
                "/v1/images/generations",
                &token,
                "application/json",
                br#"{"model":"gpt-image-1","prompt":"a cat"}"#.to_vec(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert_eq!(&body[..], IMAGES.as_bytes());
    }

    #[tokio::test]
    async fn edit_forwards_every_image_and_field() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/edits"))
            .and(body_string_contains("filename=\"a.png\""))
            .and(body_string_contains("filename=\"b.png\""))
            .and(body_string_contains("name=\"mask\""))
            .and(body_string_contains("put them in a boat"))
            .and(body_string_contains("1536x1024"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&upstream)
            .await;
        let (app, token) = app(&upstream.uri()).await;

        let body = form(&[
            ("model", None, b"gpt-image-1"),
            ("prompt", None, b"put them in a boat"),
            ("size", None, b"1536x1024"),
            ("image[]", Some("a.png"), b"AAAA"),
            ("image[]", Some("b.png"), b"BBBB"),
            ("mask", Some("m.png"), b"MMMM"),
        ]);
        let response = app
            .oneshot(post("/v1/images/edits", &token, &multipart(), body))
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn edit_without_prompt_is_rejected_before_routing() {
        let upstream = MockServer::start().await;
        let (app, token) = app(&upstream.uri()).await;

        let body = form(&[("model", None, b"gpt-image-1"), ("image", Some("a.png"), b"AAAA")]);
        let response = app
            .oneshot(post("/v1/images/edits", &token, &multipart(), body))
            .await
            .unwrap();

        assert_eq!(response.status(), 400);
        assert!(upstream.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn variation_needs_no_prompt() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/variations"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&upstream)
            .await;
        let (app, token) = app(&upstream.uri()).await;

        let body = form(&[("model", None, b"gpt-image-1"), ("image", Some("a.png"), b"AAAA")]);
        let response = app
            .oneshot(post("/v1/images/variations", &token, &multipart(), body))
            .await
            .unwrap();

        assert_eq!(response.status(), 200);
    }

    #[tokio::test]
    async fn backend_without_images_is_reported_as_unsupported_model() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&upstream)
            .await;
        let (app, token) = app(&upstream.uri()).await;

        let response = app
            .oneshot(post(
                "/v1/images/generations",
                &token,
                "application/json",
                br#"{"model":"gpt-image-1","prompt":"a cat"}"#.to_vec(),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("model_not_supported"));
    }

    #[tokio::test]
    async fn image_routes_require_auth() {
        let upstream = MockServer::start().await;
        let (app, _) = app(&upstream.uri()).await;

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/images/generations")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"gpt-image-1","prompt":"a cat"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), 401);
    }
}
