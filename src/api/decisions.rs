use axum::{
    extract::{Extension, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

use crate::api::shared::{check_model_access, error_body, metrics_user, router_error, HandlerError};
use crate::auth::admin::AuthenticatedUser;
use crate::payments::biller::BillingError;
use crate::payments::guard::{insufficient_funds_response, BillingGuard};
use crate::providers::decision::DecisionRequest;
use crate::state::AppState;

pub async fn decide(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    Json(request): Json<DecisionRequest>,
) -> Result<Response, HandlerError> {
    request
        .validate()
        .map_err(|message| (StatusCode::BAD_REQUEST, error_body(&message, "invalid_request_error")))?;

    check_model_access(&state, authenticated.user.id, &request.model).await?;

    tracing::info!(
        model = %request.model,
        questions = request.questions.len(),
        image = request.image.is_some(),
        "Received decision request"
    );

    let billing = BillingGuard::try_create(&state, Some(authenticated.user.id), &request.model, None)
        .await
        .map_err(billing_error)?;

    let result = state
        .config
        .router
        .decide(&request, Some(metrics_user(&authenticated)))
        .await;

    let usage = result.as_ref().ok().and_then(|r| r.usage()).unwrap_or_default();
    billing.finalize(usage.input_tokens, usage.output_tokens).await;
    let response = result.map_err(|e| router_error(e, "decisions"))?;

    Ok(([(header::CONTENT_TYPE, "application/json")], response.body).into_response())
}

fn billing_error(e: BillingError) -> HandlerError {
    match e {
        BillingError::InsufficientFunds { required, available } => {
            let (code, json) = insufficient_funds_response(required, available);
            (code, json.0.to_string())
        }
        e => (
            StatusCode::INTERNAL_SERVER_ERROR,
            error_body(&format!("Billing error: {e}"), "billing_error"),
        ),
    }
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

    const ANSWER: &str = r#"{"model":"NeoHorse-Jev-4B","answers":{"team":{"type":"choice","choice":"billing","confidence":0.9,"probabilities":{"billing":0.96,"other":0.04}}},"usage":{"input_tokens":40,"output_tokens":0}}"#;

    const REQUEST: &str = r#"{"model":"jev","state":"I was charged twice","questions":{"team":{"type":"choice","instructions":"Which team?","criteria":{"billing":"Charges","other":"Anything else"}}}}"#;

    async fn setup(upstream: &str) -> (Arc<AppState>, String) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let metrics_store = MetricsStore::new(1000);

        let router = Arc::new(crate::router::engine::Router::new(
            metrics_store.clone(),
            Arc::new(db.clone()),
        ));
        let provider = Arc::new(OpenAiProvider::new("neohorse", Some("neohorse"), upstream, None));
        router.add_provider(provider.clone()).await;
        router.register_route("jev", vec![provider]).await;

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

    async fn post(state: Arc<AppState>, uri: &str, token: Option<&str>, body: &str) -> axum::response::Response {
        let mut request = Request::builder()
            .uri(uri)
            .method("POST")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        create_test_app(state)
            .await
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn systemone_round_trip() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(body_string_contains(r#""model":"jev""#))
            .respond_with(ResponseTemplate::new(200).set_body_raw(ANSWER, "application/json"))
            .mount(&upstream)
            .await;

        let (state, token) = setup(&format!("{}/v1", upstream.uri())).await;
        for uri in ["/v1/systemone", "/api/alpha/decisions"] {
            let response = post(state.clone(), uri, Some(&token), REQUEST).await;
            assert_eq!(response.status(), 200, "{uri}");
            let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            assert_eq!(&body[..], ANSWER.as_bytes(), "{uri}");
        }
    }

    #[tokio::test]
    async fn requires_auth() {
        let upstream = MockServer::start().await;
        let (state, _) = setup(&upstream.uri()).await;
        let response = post(state, "/v1/systemone", None, REQUEST).await;
        assert_eq!(response.status(), 401);
    }

    #[tokio::test]
    async fn invalid_question_rejected_before_routing() {
        let upstream = MockServer::start().await;
        let (state, token) = setup(&upstream.uri()).await;
        let response = post(
            state,
            "/v1/systemone",
            Some(&token),
            r#"{"model":"jev","state":"x","questions":{"t":{"type":"score","instructions":"How bad?"}}}"#,
        )
        .await;
        assert_eq!(response.status(), 400);
        assert!(upstream.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn upstream_validation_error_keeps_its_status() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/systemone"))
            .respond_with(ResponseTemplate::new(422).set_body_string("state exceeds 2048 tokens"))
            .mount(&upstream)
            .await;

        let (state, token) = setup(&upstream.uri()).await;
        let response = post(state, "/v1/systemone", Some(&token), REQUEST).await;
        assert_eq!(response.status(), 422);
    }
}
