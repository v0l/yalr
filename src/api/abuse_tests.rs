use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::abuse::{CREATES_PER_DAY, OWNER_CONTACTS_PER_HOUR, PENDING_PER_USER};
use super::tenant_tests::{tenancy, Tenancy};

const LEAK: &str = "redis_version:7.2.4 INTERNAL-ONLY-BANNER";

fn body(slug: &str, url: &str) -> String {
    format!(r#"{{"name":"{slug}","slug":"{slug}","base_url":"{url}","provider_type":"openai"}}"#)
}

async fn create(t: &Tenancy, token: &str, slug: &str, url: &str) -> (u16, Value) {
    t.call(token, "POST", "/api/providers", &body(slug, url)).await
}

async fn leaking_upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(404).set_body_raw(format!(r#"{{"error":{{"message":"{LEAK}"}}}}"#), "application/json"))
        .mount(&server)
        .await;
    server
}

fn local(server: &MockServer) -> String {
    format!("http://catcher.example:{}/v1", server.address().port())
}

#[tokio::test]
async fn a_user_may_hold_only_a_few_pending_providers() {
    let t = tenancy().await;
    for i in 0..PENDING_PER_USER {
        let (status, reply) = create(&t, &t.alice, &format!("p{i}"), "http://8.8.8.8/v1").await;
        assert_eq!(status, 200, "{reply}");
    }
    let (status, reply) = create(&t, &t.alice, "one-more", "http://8.8.8.8/v1").await;
    assert_eq!(status, 429, "{reply}");
    assert!(reply.to_string().contains("waiting for approval"), "{reply}");
    assert_eq!(create(&t, &t.bob, "bobs", "http://8.8.8.8/v1").await.0, 200);
    assert_eq!(create(&t, &t.admin, "platform-x", "http://8.8.8.8/v1").await.0, 200);
}

#[tokio::test]
async fn a_user_may_add_only_a_few_providers_a_day_even_after_deleting_them() {
    let t = tenancy().await;
    for i in 0..CREATES_PER_DAY {
        let (status, reply) = create(&t, &t.alice, &format!("d{i}"), "http://8.8.8.8/v1").await;
        assert_eq!(status, 200, "{i}: {reply}");
        let slug = reply["slug"].as_str().unwrap().to_string();
        assert_eq!(t.call(&t.alice, "DELETE", &format!("/api/providers/{slug}"), "").await.0, 200);
    }
    let (status, reply) = create(&t, &t.alice, "late", "http://8.8.8.8/v1").await;
    assert_eq!(status, 429, "{reply}");
    assert!(reply.to_string().contains("5 providers a day"), "{reply}");
}

#[tokio::test]
async fn a_refused_url_does_not_use_up_the_daily_allowance() {
    let t = tenancy().await;
    for _ in 0..10 {
        assert_eq!(create(&t, &t.alice, "bad", "https://example.com:22/v1").await.0, 400);
    }
    assert_eq!(create(&t, &t.alice, "good", "http://8.8.8.8/v1").await.0, 200);
}

#[tokio::test]
async fn creating_a_pending_provider_never_calls_it() {
    let t = tenancy().await;
    let server = leaking_upstream().await;
    let (status, reply) = create(&t, &t.alice, "catcher", &local(&server)).await;
    assert_eq!(status, 200, "{reply}");
    t.state.config.router.settle_background().await;
    let (_, listed) = t.get(&t.admin, "/api/providers").await;
    let (_, mine) = t.get(&t.alice, "/api/providers").await;
    let (_, models) = t.get(&t.alice, "/v1/models").await;
    for reply in [listed, mine, models] {
        assert!(!reply.to_string().contains("redis_version"), "{reply}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_owner_check_is_rate_limited_and_never_shows_the_upstream_body() {
    let t = tenancy().await;
    let server = leaking_upstream().await;
    let (_, reply) = create(&t, &t.alice, "catcher", &local(&server)).await;
    let slug = reply["slug"].as_str().unwrap().to_string();

    assert_eq!(t.call(&t.bob, "POST", &format!("/api/providers/{slug}/check"), "").await.0, 404);
    for _ in 0..OWNER_CONTACTS_PER_HOUR {
        let (status, check) = t.call(&t.alice, "POST", &format!("/api/providers/{slug}/check"), "").await;
        assert_eq!(status, 200, "{check}");
        assert_eq!(check["healthy"], false);
        assert!(!check.to_string().contains("redis_version"), "{check}");
    }
    let (status, _) = t.call(&t.alice, "POST", &format!("/api/providers/{slug}/check"), "").await;
    assert_eq!(status, 429);
    assert_eq!(t.call(&t.alice, "POST", &format!("/api/providers/{slug}/verify"), "").await.0, 429);
    assert_eq!(t.get(&t.alice, &format!("/api/providers/{slug}/models")).await.0, 429);
    assert_eq!(server.received_requests().await.unwrap().len(), OWNER_CONTACTS_PER_HOUR);

    for uri in [format!("/api/providers/{slug}/listing"), "/api/metrics/health".into(), "/api/metrics".into(), "/api/providers".into()] {
        let (_, reply) = t.get(&t.alice, &uri).await;
        assert!(!reply.to_string().contains("redis_version"), "{uri}: {reply}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), OWNER_CONTACTS_PER_HOUR);
}

#[tokio::test]
async fn the_owners_model_list_error_does_not_echo_the_upstream() {
    let t = tenancy().await;
    let server = leaking_upstream().await;
    let (_, reply) = create(&t, &t.alice, "catcher", &local(&server)).await;
    let slug = reply["slug"].as_str().unwrap().to_string();
    let (status, models) = t.get(&t.alice, &format!("/api/providers/{slug}/models")).await;
    assert_eq!(status, 500);
    assert!(!models.to_string().contains("redis_version"), "{models}");
}
