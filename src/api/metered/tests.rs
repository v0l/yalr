use crate::api::server::create_test_app;
use crate::config::{AppConfig, DefaultPricingConfig, PaymentsConfig};
use crate::db::{Database, Filter, NewApiKey, Summary};
use crate::metrics::MetricsStore;
use crate::payments::{balance::BalanceService, pricing::PricingResolver, PaymentsState};
use crate::providers::OpenAiProvider;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::util::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-metered-test";
const FUNDS_MSAT: i64 = 10_000_000;

struct Harness {
    state: Arc<AppState>,
    upstream: MockServer,
    user_id: i64,
}

async fn harness(model: &str) -> Harness {
    let upstream = MockServer::start().await;
    let db = Arc::new(Database::test().await);
    let metrics_store = Arc::new(MetricsStore::new(100));
    let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
    let provider = Arc::new(OpenAiProvider::new("up", Some("up"), &format!("{}/v1", upstream.uri()), Some("k")));
    router.add_provider(provider.clone()).await;
    router.register_route(model, vec![provider]).await;
    let balance_service = Arc::new(BalanceService::new(db.clone()));
    let payments = PaymentsState {
        config: PaymentsConfig::default(),
        balance_service: balance_service.clone(),
        lightning_service: None,
        pricing_resolver: Arc::new(PricingResolver::new(db.clone(), DefaultPricingConfig::default())),
        settlement_listener: None,
    };
    let state = Arc::new(AppState {
        config: AppConfig {
            db: db.clone(),
            router,
            auth_config: Default::default(),
            payments_config: None,
            discovery_config: None,
            admin_ui_path: String::new(),
            host: String::new(),
            port: 0,
        },
        metrics_emitter: metrics_store.emitter().clone(),
        metrics_store,
        session_store: Arc::new(crate::auth::admin::SessionStore::new()),
        db: db.clone(),
        payments_state: Some(Arc::new(payments)),
        oauth_pending: Default::default(),
        model_cache: Default::default(),
        modality_cache: Default::default(),
        limits_cache: Default::default(),
    });
    let user = db.add_test_user("buyer", false).await;
    db.create_api_key(NewApiKey {
        key_hash: &crate::auth::api_key::hash_api_key(KEY),
        name: "main",
        user_id: user.id,
        last_four: "test",
        expires_at: None,
    })
    .await
    .unwrap();
    db.execute_raw(&format!(
        "INSERT INTO model_pricing (model_name, is_advertised, price_per_1m_input_sats, price_per_1m_output_sats, \
         price_per_request_sats) VALUES ('{model}', 1, 1000, 4000, 2)"
    ))
    .await
    .unwrap();
    balance_service.credit(user.id, FUNDS_MSAT, "deposit", "seed").await.unwrap();
    Harness { state, upstream, user_id: user.id }
}

impl Harness {
    async fn send(&self, uri: &str, content_type: &str, body: Vec<u8>) -> (StatusCode, Vec<u8>) {
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {KEY}"))
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let response = create_test_app(self.state.clone()).await.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 26).await.unwrap();
        (status, bytes.to_vec())
    }

    async fn json(&self, uri: &str, body: &str) -> (StatusCode, Vec<u8>) {
        self.send(uri, "application/json", body.as_bytes().to_vec()).await
    }

    async fn spent_msat(&self) -> i64 {
        let balance = &self.state.payments_state.as_ref().unwrap().balance_service;
        FUNDS_MSAT - balance.get_balance(self.user_id).await.unwrap()
    }

    async fn logged(&self, endpoint: &str) -> Summary {
        for _ in 0..100 {
            let filter = Filter { endpoint: Some(endpoint), ..Default::default() };
            if let Some(row) = self.state.db.list_request_logs(10, filter).await.unwrap().into_iter().next() {
                if row.cost_msat.is_some() || row.status != "ok" {
                    return row;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("no request log row for {endpoint}");
    }

    async fn bodies(&self, row: &Summary) -> (serde_json::Value, serde_json::Value) {
        let bodies = self.state.db.request_log_bodies(row.id).await.unwrap().expect("bodies stored");
        let decode = |b: Option<Vec<u8>>| {
            let raw = crate::request_log::codec::unpack(&b.unwrap()).unwrap();
            serde_json::from_slice(&raw).unwrap_or(serde_json::Value::String(String::from_utf8_lossy(&raw).into()))
        };
        (decode(bodies.client_request_packed), decode(bodies.response_packed))
    }
}

fn multipart(fields: &[(&str, &str)], files: &[(&str, &str, &str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "metered-boundary";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").bytes());
    }
    for (name, file_name, content_type, data) in files {
        body.extend(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            )
            .bytes(),
        );
        body.extend_from_slice(data);
        body.extend(b"\r\n");
    }
    body.extend(format!("--{boundary}--\r\n").bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

const PER_REQUEST_MSAT: i64 = 2_000;

fn token_cost_msat(input: i64, output: i64) -> i64 {
    (input * 1000 + output * 4000 + 999) / 1000 + PER_REQUEST_MSAT
}

#[tokio::test]
async fn responses_api_is_served_logged_and_billed() {
    let h = harness("resp").await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "resp_1", "object": "response", "created_at": 0, "model": "resp", "status": "completed",
            "output": [], "usage": {"input_tokens": 1000, "output_tokens": 500, "total_tokens": 1500,
            "input_tokens_details": {"cached_tokens": 0}, "output_tokens_details": {"reasoning_tokens": 0}}
        })))
        .mount(&h.upstream)
        .await;
    let (status, body) = h.json("/v1/responses", r#"{"model":"resp","input":"hi"}"#).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let row = h.logged("/v1/responses").await;
    assert_eq!((row.status.as_str(), row.provider.as_deref()), ("ok", Some("up")));
    assert_eq!((row.prompt_tokens, row.completion_tokens), (Some(1000), Some(500)));
    let cost = token_cost_msat(1000, 500);
    assert_eq!(row.cost_msat, Some(cost));
    assert_eq!(h.spent_msat().await, cost);
    let (request, response) = h.bodies(&row).await;
    assert_eq!(request["input"], "hi");
    assert_eq!(response["id"], "resp_1");
}

#[tokio::test]
async fn decisions_are_logged_with_their_cost() {
    let h = harness("jev").await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"model":"jev","answers":{"t":{"type":"choice","choice":"a"}},"usage":{"input_tokens":400,"output_tokens":20}}"#,
            "application/json",
        ))
        .mount(&h.upstream)
        .await;
    let body = r#"{"model":"jev","state":"s","questions":{"t":{"type":"choice","instructions":"i","criteria":{"a":"A","b":"B"}}}}"#;
    let (status, _) = h.json("/v1/systemone", body).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/systemone").await;
    assert_eq!((row.prompt_tokens, row.completion_tokens), (Some(400), Some(20)));
    let cost = token_cost_msat(400, 20);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
    let (request, response) = h.bodies(&row).await;
    assert_eq!(request["state"], "s");
    assert_eq!(response["answers"]["t"]["choice"], "a");
}

#[tokio::test]
async fn transcriptions_log_the_uploaded_audio_and_bill_reported_tokens() {
    let h = harness("gpt-4o-transcribe").await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "text": "hello", "usage": {"type": "tokens", "input_tokens": 300, "output_tokens": 10}
        })))
        .mount(&h.upstream)
        .await;
    let audio = vec![7u8; 4096];
    let (content_type, body) = multipart(&[("model", "gpt-4o-transcribe"), ("language", "en")], &[("file", "a.wav", "audio/wav", &audio)]);
    let (status, _) = h.send("/v1/audio/transcriptions", &content_type, body).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/audio/transcriptions").await;
    let cost = token_cost_msat(300, 10);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
    let (request, response) = h.bodies(&row).await;
    assert_eq!(request["files"][0]["bytes"], 4096);
    assert_eq!(request["files"][0]["content_type"], "audio/wav");
    assert_eq!(request["fields"][1], serde_json::json!(["language", "en"]));
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, request["files"][0]["base64"].as_str().unwrap()).unwrap();
    assert_eq!(decoded, audio);
    assert_eq!(response["text"], "hello");
}

#[tokio::test]
async fn whisper_without_usage_pays_the_request_fee() {
    let h = harness("whisper-1").await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/translations"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("hello", "text/plain"))
        .mount(&h.upstream)
        .await;
    let (content_type, body) = multipart(&[("model", "whisper-1")], &[("file", "a.mp3", "audio/mpeg", b"mp3")]);
    let (status, _) = h.send("/v1/audio/translations", &content_type, body).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/audio/translations").await;
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(PER_REQUEST_MSAT), PER_REQUEST_MSAT));
    assert_eq!(h.bodies(&row).await.1, "hello");
}

#[tokio::test]
async fn speech_audio_is_streamed_to_the_client_and_logged_in_full() {
    let h = harness("tts-1").await;
    let audio: Vec<u8> = (0..20_000u32).map(|n| (n % 251) as u8).collect();
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(audio.clone(), "audio/mpeg"))
        .mount(&h.upstream)
        .await;
    let (status, body) = h.json("/v1/audio/speech", r#"{"model":"tts-1","input":"say this","voice":"alloy"}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, audio);
    let row = h.logged("/v1/audio/speech").await;
    assert_eq!((row.status.as_str(), row.cost_msat), ("ok", Some(PER_REQUEST_MSAT)));
    assert_eq!(h.spent_msat().await, PER_REQUEST_MSAT);
    let (request, response) = h.bodies(&row).await;
    assert_eq!(request["input"], "say this");
    assert_eq!(response["content_type"], "audio/mpeg");
    assert_eq!(response["bytes"], 20_000);
}

#[tokio::test]
async fn image_generations_bill_gpt_image_tokens() {
    let h = harness("gpt-image-1").await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "created": 0, "data": [{"b64_json": "aGk="}],
            "usage": {"input_tokens": 50, "output_tokens": 4160, "total_tokens": 4210,
                      "input_tokens_details": {"text_tokens": 50, "image_tokens": 0}}
        })))
        .mount(&h.upstream)
        .await;
    let (status, _) = h.json("/v1/images/generations", r#"{"model":"gpt-image-1","prompt":"a cat"}"#).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/images/generations").await;
    assert_eq!((row.prompt_tokens, row.completion_tokens), (Some(50), Some(4160)));
    let cost = token_cost_msat(50, 4160);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
    let (request, response) = h.bodies(&row).await;
    assert_eq!(request["prompt"], "a cat");
    assert_eq!(response["data"][0]["b64_json"], "aGk=");
}

#[tokio::test]
async fn image_edits_log_every_uploaded_file() {
    let h = harness("dall-e-2").await;
    Mock::given(method("POST"))
        .and(path("/v1/images/edits"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"created": 0, "data": [{"url": "u"}]})))
        .mount(&h.upstream)
        .await;
    let (content_type, body) = multipart(
        &[("model", "dall-e-2"), ("prompt", "add a hat")],
        &[("image", "in.png", "image/png", b"png"), ("mask", "m.png", "image/png", b"mask")],
    );
    let (status, _) = h.send("/v1/images/edits", &content_type, body).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/images/edits").await;
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(PER_REQUEST_MSAT), PER_REQUEST_MSAT));
    let (request, _) = h.bodies(&row).await;
    let names: Vec<&str> = request["files"].as_array().unwrap().iter().map(|f| f["field"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["image", "mask"]);
}

#[tokio::test]
async fn failed_upstream_is_logged_and_not_billed() {
    let h = harness("jev").await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(r#"{"error":{"message":"bad question"}}"#, "application/json"))
        .mount(&h.upstream)
        .await;
    let body = r#"{"model":"jev","state":"s","questions":{"t":{"type":"choice","instructions":"i","criteria":{"a":"A","b":"B"}}}}"#;
    let (status, _) = h.json("/v1/systemone", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let row = h.logged("/v1/systemone").await;
    assert_eq!(row.status, "error");
    assert!(row.error.unwrap().contains("bad question"));
    assert_eq!(row.attempts, 1);
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn an_unfunded_user_is_rejected_and_the_rejection_is_logged() {
    let h = harness("tts-1").await;
    h.state.db.execute_raw(&format!("UPDATE user_balances SET balance_msat = 0 WHERE user_id = {}", h.user_id)).await.unwrap();
    let (status, _) = h.json("/v1/audio/speech", r#"{"model":"tts-1","input":"x"}"#).await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    let row = h.logged("/v1/audio/speech").await;
    assert_eq!(row.status, "rejected");
}

#[tokio::test]
async fn streamed_image_generation_bills_usage_from_the_final_event() {
    let h = harness("gpt-image-1").await;
    let sse = "event: image_generation.partial_image\ndata: {\"type\":\"image_generation.partial_image\",\"b64_json\":\"cGFy\"}\n\n\
               event: image_generation.completed\ndata: {\"type\":\"image_generation.completed\",\"b64_json\":\"ZG9uZQ==\",\
               \"usage\":{\"input_tokens\":20,\"output_tokens\":1000,\"total_tokens\":1020}}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&h.upstream)
        .await;
    let (status, body) = h.json("/v1/images/generations", r#"{"model":"gpt-image-1","prompt":"p","stream":true}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, sse.as_bytes());
    let row = h.logged("/v1/images/generations").await;
    assert!(row.stream);
    let cost = token_cost_msat(20, 1000);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
}

#[tokio::test]
async fn a_client_that_disconnects_mid_stream_is_still_logged_and_billed() {
    let h = harness("tts-1").await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1u8; 1 << 20], "audio/mpeg"))
        .mount(&h.upstream)
        .await;
    let request = Request::builder()
        .method("POST")
        .uri("/v1/audio/speech")
        .header("authorization", format!("Bearer {KEY}"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"model":"tts-1","input":"x"}"#))
        .unwrap();
    let response = create_test_app(h.state.clone()).await.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    let row = h.logged("/v1/audio/speech").await;
    assert_eq!(row.status, "error");
    assert_eq!(row.error.as_deref(), Some("client disconnected"));
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(PER_REQUEST_MSAT), PER_REQUEST_MSAT));
}

#[tokio::test]
async fn chat_rows_carry_their_cost() {
    let h = harness("chat").await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c1", "object": "chat.completion", "created": 0, "model": "chat",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 2000, "completion_tokens": 100, "total_tokens": 2100}
        })))
        .mount(&h.upstream)
        .await;
    let (status, _) = h.json("/v1/chat/completions", r#"{"model":"chat","messages":[{"role":"user","content":"hi"}]}"#).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/chat/completions").await;
    let cost = token_cost_msat(2000, 100);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
}
