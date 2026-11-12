use super::routing_modalities::tests::{app, send};

const CONFIG: &str = r#""strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5"#;

async fn create_config(app: &axum::Router, token: &str, name: &str) {
    let (status, body) =
        send(app, "POST", "/api/routing-configs", token, &format!(r#"{{"name":"{name}",{CONFIG}}}"#)).await;
    assert_eq!(status, 200, "{body}");
}

async fn add_child(app: &axum::Router, token: &str, parent: i64, child: i64) -> (u16, String) {
    send(
        app,
        "POST",
        "/api/routing-configs/providers",
        token,
        &format!(r#"{{"routing_config_id":{parent},"config_id":{child},"weight":10,"is_active":true}}"#),
    )
    .await
}

#[tokio::test]
async fn a_config_can_be_added_as_a_member_and_is_listed_as_one() {
    let (app, token) = app().await;
    create_config(&app, &token, "code-high").await;
    create_config(&app, &token, "code").await;

    let (status, body) = add_child(&app, &token, 1, 2).await;
    assert_eq!(status, 200, "{body}");

    let (_, body) = send(&app, "GET", "/api/routing-configs", &token, "").await;
    assert!(body.contains(r#""kind":"config""#), "{body}");
    assert!(body.contains(r#""config_id":2"#), "{body}");
    assert!(body.contains(r#""config_name":"code""#), "{body}");
    assert!(!body.contains("provider_id"), "{body}");
}

#[tokio::test]
async fn self_reference_and_cycles_are_refused_with_a_400() {
    let (app, token) = app().await;
    create_config(&app, &token, "a").await;
    create_config(&app, &token, "b").await;

    let (status, body) = add_child(&app, &token, 1, 1).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("cannot contain itself"), "{body}");

    assert_eq!(add_child(&app, &token, 1, 2).await.0, 200);
    let (status, body) = add_child(&app, &token, 2, 1).await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("cycle"), "{body}");
}

#[tokio::test]
async fn a_member_must_name_exactly_one_target() {
    let (app, token) = app().await;
    create_config(&app, &token, "a").await;
    let (status, _) = send(
        &app,
        "POST",
        "/api/routing-configs/providers",
        &token,
        r#"{"routing_config_id":1,"weight":1,"is_active":true}"#,
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = add_child(&app, &token, 1, 99).await;
    assert_eq!(status, 404);
}
