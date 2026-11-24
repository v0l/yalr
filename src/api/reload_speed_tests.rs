use super::routing_modalities::tests::{app, send};
use std::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn saving_a_provider_does_not_wait_for_a_hung_model_list() {
    let (app, token) = app().await;
    let hung = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&hung)
        .await;
    let body = format!(
        r#"{{"name":"hung","slug":"hung","base_url":"{}/v1","api_key":"k","provider_type":"llamacpp"}}"#,
        hung.uri()
    );
    let started = Instant::now();
    let (status, body) = send(&app, "POST", "/api/providers", &token, &body).await;
    assert_eq!(status, 200, "{body}");
    assert!(started.elapsed() < Duration::from_millis(500), "create took {:?}", started.elapsed());

    let started = Instant::now();
    let (status, body) = send(&app, "PUT", "/api/providers/hung", &token, r#"{"max_concurrency":4}"#).await;
    assert_eq!(status, 200, "{body}");
    assert!(started.elapsed() < Duration::from_millis(500), "update took {:?}", started.elapsed());
}
