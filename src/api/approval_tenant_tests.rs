use serde_json::Value;

use super::tenant_tests::tenancy;

fn ids(body: &Value) -> Vec<i64> {
    body.as_array().unwrap().iter().map(|k| k["id"].as_i64().unwrap()).collect()
}

#[tokio::test]
async fn a_user_manages_only_their_own_api_keys() {
    let t = tenancy().await;
    let (status, made) = t.call(&t.alice, "POST", "/api/api-keys", r#"{"name":"laptop"}"#).await;
    assert_eq!(status, 200, "{made}");
    assert!(made["key"].as_str().unwrap().len() > 20);
    let id = made["id"].as_i64().unwrap();

    assert_eq!(ids(&t.get(&t.alice, "/api/api-keys").await.1), vec![id]);
    assert!(ids(&t.get(&t.bob, "/api/api-keys").await.1).is_empty());
    assert_eq!(t.call(&t.bob, "DELETE", &format!("/api/api-keys/{id}"), "").await.0, 404);
    assert_eq!(t.call(&t.alice, "DELETE", &format!("/api/api-keys/{id}"), "").await.0, 200);
    assert!(ids(&t.get(&t.alice, "/api/api-keys").await.1).is_empty());
}

#[tokio::test]
async fn a_new_user_provider_shows_pending_to_its_owner_and_to_admins_only() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let mine = t.get(&t.alice, "/api/providers").await.1;
    assert_eq!(mine["providers"][0]["status"], "pending");
    assert!(mine["providers"][0]["verifications"].as_array().unwrap().is_empty());

    let all = t.get(&t.admin, "/api/providers").await.1;
    let row = all["providers"].as_array().unwrap().iter().find(|p| p["slug"] == slug.as_str()).unwrap().clone();
    assert_eq!(row["status"], "pending");

    assert_eq!(t.get(&t.alice, &format!("/api/providers/{slug}/verifications")).await.0, 200);
    assert_eq!(t.get(&t.bob, &format!("/api/providers/{slug}/verifications")).await.0, 404);
    assert_eq!(t.call(&t.bob, "POST", &format!("/api/providers/{slug}/verify"), "").await.0, 404);
}

#[tokio::test]
async fn only_admins_approve_or_mark_references() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    let approve = r#"{"action":"approve"}"#;
    assert_eq!(t.call(&t.alice, "PUT", &format!("/api/providers/{slug}/status"), approve).await.0, 403);
    assert_eq!(t.call(&t.alice, "PUT", &format!("/api/providers/{slug}/reference"), r#"{"is_reference":true}"#).await.0, 403);

    let (status, body) = t.call(&t.admin, "PUT", &format!("/api/providers/{slug}/status"), approve).await;
    assert_eq!((status, body["status"].as_str()), (200, Some("approved")));
    let (status, body) = t.call(&t.admin, "PUT", &format!("/api/providers/{slug}/status"), r#"{"action":"reject","reason":"wrong model"}"#).await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "suspended");
    assert_eq!(body["status_locked"], true);
    assert_eq!(t.get(&t.alice, "/api/providers").await.1["providers"][0]["status_reason"], "Rejected: wrong model");
}

#[tokio::test]
async fn editing_an_approved_providers_url_sends_it_back_to_pending() {
    let t = tenancy().await;
    let slug = t.alice_provider().await;
    t.call(&t.admin, "PUT", &format!("/api/providers/{slug}/status"), r#"{"action":"approve"}"#).await;

    let (status, renamed) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"name":"renamed"}"#).await;
    assert_eq!((status, renamed["status"].as_str()), (200, Some("approved")));
    let (status, moved) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"base_url":"http://8.8.4.4/v1"}"#).await;
    assert_eq!((status, moved["status"].as_str()), (200, Some("pending")), "{moved}");
}
