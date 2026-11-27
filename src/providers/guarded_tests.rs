use super::*;
use crate::providers::egress::{as_caller, Caller, Lookup};
use std::net::IpAddr;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Flipping(Mutex<Vec<Vec<IpAddr>>>);

#[async_trait]
impl Lookup for Flipping {
    async fn lookup(&self, _host: &str) -> std::io::Result<Vec<IpAddr>> {
        let mut answers = self.0.lock().unwrap();
        Ok(if answers.len() > 1 { answers.remove(0) } else { answers[0].clone() })
    }
}

fn flipping(answers: &[&str]) -> Arc<Flipping> {
    Arc::new(Flipping(Mutex::new(answers.iter().map(|ip| vec![ip.parse().unwrap()]).collect())))
}

fn loopback_is_public(ip: IpAddr) -> bool {
    ip == IpAddr::from([127, 0, 0, 1])
}

fn record(base_url: &str, status: &str, kind: crate::db::ProviderType) -> crate::db::Provider {
    crate::db::Provider {
        id: 1,
        name: "u7/gpu".into(),
        slug: "u7-gpu".into(),
        base_url: base_url.into(),
        api_key: Some("sk-k".into()),
        provider_type: kind,
        owner_user_id: Some(7),
        status: status.into(),
        ..crate::db::Provider::blank_for_tests()
    }
}

fn build(base_url: &str, status: &str, kind: crate::db::ProviderType, lookup: Arc<Flipping>) -> Arc<dyn Provider> {
    let record = record(base_url, status, kind);
    let access: Access = Default::default();
    access.write().unwrap().insert(record.slug.clone(), (record.owner_user_id, record.status.clone()));
    let egress = Egress::with_lookup(lookup, loopback_is_public);
    crate::providers::create_user_provider(&record, &egress, access).unwrap()
}

fn chat() -> ChatRequest {
    serde_json::from_str(r#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#).unwrap()
}

async fn upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"object":"list","data":[{"id":"m","object":"model","created":0,"owned_by":"x"}]}"#, "application/json"))
        .mount(&server)
        .await;
    server
}

const KINDS: &[crate::db::ProviderType] = &[
    crate::db::ProviderType::OpenAi,
    crate::db::ProviderType::LlamaCpp,
    crate::db::ProviderType::Vllm,
    crate::db::ProviderType::Ollama,
    crate::db::ProviderType::Routstr,
    crate::db::ProviderType::OpenRouter,
    crate::db::ProviderType::Anthropic,
    crate::db::ProviderType::Ppq,
];

#[tokio::test]
async fn requests_to_a_private_host_never_leave() {
    let p = build("http://10.1.2.3:8080/v1", "approved", crate::db::ProviderType::OpenAi, flipping(&["10.1.2.3"]));
    let err = p.list_models().await.unwrap_err().to_string();
    assert!(err.contains("non-public"), "{err}");
    assert!(p.health_check().await.is_err());
}

#[tokio::test]
async fn every_kind_connects_only_to_the_checked_address_when_dns_rebinds() {
    let server = upstream().await;
    let url = format!("http://rebind.invalid:{}/v1", server.address().port());
    for kind in KINDS {
        let p = build(&url, "approved", *kind, flipping(&["127.0.0.1", "10.42.0.5"]));
        let _ = p.list_models().await;
        let _ = p.health_check().await;
        let mut stream = p.chat_completions_stream(&chat()).unwrap();
        while stream.next().await.is_some() {}
        let _ = p.chat_completions(&chat()).await;
    }
    let reached = server.received_requests().await.unwrap().len();
    assert_eq!(reached, 0, "a rebound name reached the upstream {reached} times");
}

#[tokio::test]
async fn an_approved_provider_reaches_its_checked_address() {
    let server = upstream().await;
    let url = format!("http://pinned.invalid:{}/v1", server.address().port());
    let p = build(&url, "approved", crate::db::ProviderType::OpenAi, flipping(&["127.0.0.1"]));
    assert_eq!(p.list_models().await.unwrap()[0].id, "m");
}

#[tokio::test]
async fn a_redirect_to_a_private_address_is_not_followed() {
    let inside = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).mount(&inside).await;
    let outside = MockServer::start().await;
    let location = format!("http://10.0.0.5:{}/v1/models", inside.address().port());
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", location.as_str()))
        .mount(&outside)
        .await;
    let url = format!("http://outside.invalid:{}/v1", outside.address().port());
    for kind in KINDS {
        let p = build(&url, "approved", *kind, flipping(&["127.0.0.1"]));
        assert!(!matches!(p.health_check().await, Ok(true)), "{kind:?}");
        let _ = p.list_models().await;
    }
    assert!(inside.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_pending_provider_is_contacted_only_by_its_owner_or_a_verification() {
    let server = upstream().await;
    let url = format!("http://pending.invalid:{}/v1", server.address().port());
    let p = build(&url, "pending", crate::db::ProviderType::OpenAi, flipping(&["127.0.0.1"]));
    assert_eq!(p.list_models().await.unwrap_err().to_string(), format!("Unsupported by provider: {NOT_APPROVED}"));
    assert!(p.health_check().await.is_err());
    assert!(p.chat_completions_stream(&chat()).is_err());
    assert!(as_caller(Caller::user(8, false), p.list_models()).await.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(as_caller(Caller::user(7, false), p.list_models()).await.is_ok());
    assert!(as_caller(Caller::verification(7), p.health_check()).await.is_ok());
    assert!(as_caller(Caller::user(1, true), p.health_check()).await.is_ok());
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn the_upstream_body_never_reaches_the_owner() {
    const SECRET: &str = "redis_version:7.2.4 INTERNAL-ONLY-BANNER";
    let server = MockServer::start().await;
    for status in [400u16, 401, 404] {
        Mock::given(method("POST")).and(path(format!("/s{status}/chat/completions")))
            .respond_with(ResponseTemplate::new(status).set_body_raw(format!(r#"{{"error":{{"message":"{SECRET}"}}}}"#), "application/json"))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path(format!("/s{status}/models")))
            .respond_with(ResponseTemplate::new(status).set_body_string(SECRET))
            .mount(&server)
            .await;
    }
    Mock::given(method("POST")).and(path("/plain/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SECRET))
        .mount(&server)
        .await;
    let port = server.address().port();
    for prefix in ["s400", "s401", "s404", "plain"] {
        let url = format!("http://leak.invalid:{port}/{prefix}");
        for kind in KINDS {
            let p = build(&url, "approved", *kind, flipping(&["127.0.0.1"]));
            let mut errors = vec![];
            if let Err(e) = p.chat_completions(&chat()).await {
                errors.push(e.to_string());
            }
            if let Err(e) = p.list_models().await {
                errors.push(e.to_string());
            }
            if let Ok(mut stream) = p.chat_completions_stream(&chat()) {
                while let Some(item) = stream.next().await {
                    if let Err(e) = item {
                        errors.push(e.to_string());
                    }
                }
            }
            for text in errors {
                assert!(!text.contains("redis_version") && !text.contains("INTERNAL"), "{kind:?} {prefix}: {text}");
            }
        }
    }
}

#[test]
fn sanitize_keeps_the_class_and_drops_the_body() {
    let e = sanitize(ProviderError::ServerError { message: "secret maximum context length exceeded".into(), status_code: Some(400) });
    assert_eq!(e.to_string(), "Server error: upstream returned HTTP 400: maximum context length (status: Some(400))");
    assert!(e.is_client_error());
    let e = sanitize(ProviderError::RateLimit { retry_after_ms: 5, message: "secret".into() });
    assert_eq!(e.retry_after_ms(), Some(5));
    assert!(!e.to_string().contains("secret"));
    let e = sanitize(ProviderError::Other("error sending request for url (http://x/secret): connection refused".to_string().into()));
    assert!(e.is_connection_failure());
    assert!(!e.to_string().contains("secret"));
    let e = sanitize(ProviderError::ServerError { message: "secret overloaded".into(), status_code: Some(529) });
    assert!(!e.is_retryable_transient());
    let e = sanitize(ProviderError::ServerError { message: "redis_version:7 secret".into(), status_code: Some(502) });
    assert_eq!(e.to_string(), "Server error: upstream returned HTTP 502 (status: Some(502))");
    assert!(e.is_retryable_transient());
}
