use super::routing_modalities::tests::{app, send};

const CONFIG: &str = r#""strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5"#;

fn provider_json(extra: &str) -> String {
    format!(r#"{{"name":"up","slug":"up","base_url":"http://127.0.0.1:1/v1","api_key":"k","provider_type":"openai"{extra}}}"#)
}

#[tokio::test]
async fn provider_extra_body_round_trips_and_rejects_non_objects() {
    let (app, token) = app().await;

    let (status, body) = send(
        &app,
        "POST",
        "/api/providers",
        &token,
        &provider_json(r#","extra_body":["nope"]"#),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("JSON object"), "{body}");

    let (status, body) = send(
        &app,
        "POST",
        "/api/providers",
        &token,
        &provider_json(r#","extra_body":{"provider":{"order":["x"],"allow_fallbacks":false}}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (_, body) = send(&app, "GET", "/api/providers", &token, "").await;
    assert!(body.contains(r#""extra_body":{"provider":{"allow_fallbacks":false,"order":["x"]}}"#), "{body}");

    let (status, body) = send(&app, "PUT", "/api/providers/up", &token, r#"{"extra_body":"text"}"#).await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = send(&app, "PUT", "/api/providers/up", &token, r#"{"extra_body":null}"#).await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = send(&app, "GET", "/api/providers", &token, "").await;
    assert!(body.contains(r#""extra_body":{"provider""#), "null leaves it untouched: {body}");

    let (status, body) = send(&app, "PUT", "/api/providers/up", &token, r#"{"extra_body":{}}"#).await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = send(&app, "GET", "/api/providers", &token, "").await;
    assert!(!body.contains("extra_body"), "{body}");
}

#[tokio::test]
async fn routing_entry_extra_body_round_trips_and_rejects_non_objects() {
    let (app, token) = app().await;
    let (status, _) = send(&app, "POST", "/api/providers", &token, &provider_json("")).await;
    assert_eq!(status, 200);
    let (status, body) = send(&app, "POST", "/api/routing-configs", &token, &format!(r#"{{"name":"m",{CONFIG}}}"#)).await;
    assert_eq!(status, 200, "{body}");

    let entry = r#"{"routing_config_id":1,"provider_id":1,"weight":1,"is_active":true"#;
    let (status, body) =
        send(&app, "POST", "/api/routing-configs/providers", &token, &format!(r#"{entry},"extra_body":5}}"#)).await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = send(
        &app,
        "POST",
        "/api/routing-configs/providers",
        &token,
        &format!(r#"{entry},"extra_body":{{"provider":{{"sort":"price"}}}}}}"#),
    )
    .await;
    assert_eq!(status, 200, "{body}");

    let (_, body) = send(&app, "GET", "/api/routing-configs", &token, "").await;
    assert!(body.contains(r#""extra_body":{"provider":{"sort":"price"}}"#), "{body}");

    let (status, body) =
        send(&app, "PUT", "/api/routing-configs/providers/1", &token, r#"{"extra_body":[]}"#).await;
    assert_eq!(status, 400, "{body}");

    let (status, body) =
        send(&app, "PUT", "/api/routing-configs/providers/1", &token, r#"{"extra_body":{}}"#).await;
    assert_eq!(status, 200, "{body}");
    let (_, body) = send(&app, "GET", "/api/routing-configs", &token, "").await;
    assert!(!body.contains("extra_body"), "{body}");
}
