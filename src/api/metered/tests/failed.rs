use super::*;

const CHAT: &str = r#"{"model":"chat","messages":[{"role":"user","content":"hi"}]}"#;
const CHAT_STREAM: &str = r#"{"model":"chat","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;

fn completion(prompt: u32, completion: u32) -> serde_json::Value {
    serde_json::json!({
        "id": "c1", "object": "chat.completion", "created": 0, "model": "chat",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion}
    })
}

async fn upstream_fails(h: &Harness, uri: &'static str, status: u16) {
    Mock::given(method("POST"))
        .and(path(uri))
        .respond_with(ResponseTemplate::new(status).set_body_raw(r#"{"error":{"message":"boom"}}"#, "application/json"))
        .mount(&h.upstream)
        .await;
}

#[tokio::test]
async fn a_failed_chat_stream_is_not_charged_the_request_fee() {
    let h = harness("chat").await;
    upstream_fails(&h, "/v1/chat/completions", 500).await;
    h.json("/v1/chat/completions", CHAT_STREAM).await;
    let row = h.logged("/v1/chat/completions").await;
    assert_eq!(row.status, "error");
    assert_eq!(row.cost_msat.unwrap_or(0), 0);
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn a_failed_chat_completion_is_not_charged_the_request_fee() {
    let h = harness("chat").await;
    upstream_fails(&h, "/v1/chat/completions", 400).await;
    let (status, _) = h.json("/v1/chat/completions", CHAT).await;
    assert_ne!(status, StatusCode::OK);
    let row = h.logged("/v1/chat/completions").await;
    assert_eq!(row.cost_msat.unwrap_or(0), 0);
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn a_successful_chat_stream_pays_the_fee_and_tokens() {
    let h = harness("chat").await;
    let sse = "data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"chat\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2000,\"completion_tokens\":100,\"total_tokens\":2100}}\n\n\
               data: {\"id\":\"c\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"chat\",\"choices\":[],\"usage\":{\"prompt_tokens\":2000,\"completion_tokens\":100,\"total_tokens\":2100}}\n\n\
               data: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .mount(&h.upstream)
        .await;
    let (status, _) = h.json("/v1/chat/completions", CHAT_STREAM).await;
    assert_eq!(status, StatusCode::OK);
    let row = h.logged("/v1/chat/completions").await;
    let cost = token_cost_msat(2000, 100);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
}

#[tokio::test]
async fn a_failed_responses_request_is_not_charged() {
    let h = harness("resp").await;
    upstream_fails(&h, "/v1/responses", 400).await;
    h.json("/v1/responses", r#"{"model":"resp","input":"hi"}"#).await;
    let row = h.logged("/v1/responses").await;
    assert_eq!(row.cost_msat.unwrap_or(0), 0);
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn a_failed_speech_stream_is_not_charged() {
    let h = harness("tts-1").await;
    upstream_fails(&h, "/v1/audio/speech", 500).await;
    h.json("/v1/audio/speech", r#"{"model":"tts-1","input":"x"}"#).await;
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn a_failed_image_request_is_not_charged() {
    let h = harness("dall-e-2").await;
    upstream_fails(&h, "/v1/images/generations", 500).await;
    h.json("/v1/images/generations", r#"{"model":"dall-e-2","prompt":"p"}"#).await;
    assert_eq!(h.spent_msat().await, 0);
}

#[tokio::test]
async fn chat_failover_charges_one_request_fee() {
    let h = harness("chat").await;
    let down: Arc<dyn crate::providers::Provider> =
        Arc::new(OpenAiProvider::new("down", Some("down"), "http://127.0.0.1:9/v1", Some("k")));
    let up: Arc<dyn crate::providers::Provider> =
        Arc::new(OpenAiProvider::new("up2", Some("up2"), &format!("{}/v1", h.upstream.uri()), Some("k")));
    let router = &h.state.config.router;
    router.add_provider(down.clone()).await;
    router.add_provider(up.clone()).await;
    router.register_route("chat", vec![down, up]).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion(2000, 100)))
        .mount(&h.upstream)
        .await;
    let (status, body) = h.json("/v1/chat/completions", CHAT).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let row = h.logged("/v1/chat/completions").await;
    assert!(row.attempts >= 1);
    let cost = token_cost_msat(2000, 100);
    assert_eq!((row.cost_msat, h.spent_msat().await), (Some(cost), cost));
}
