//! End to end check that `/v1/chat/completions` with `stream: true` forwards
//! chunks as the upstream produces them rather than all at once at the end.

use crate::api::server::create_test_app;
use crate::auth::admin::SessionStore;
use crate::db::{Database, NewUser, UserType};
use crate::metrics::MetricsStore;
use crate::providers::OpenAiProvider;
use crate::state::AppState;
use axum::{body::Body, routing::post};
use futures::StreamExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

const GAP: Duration = Duration::from_millis(400);
const CHUNKS: usize = 5;

fn sse(delta: &str) -> String {
    format!(
        "data: {{\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"slow\",\"choices\":[{{\"index\":0,\"delta\":{{{delta}}},\"finish_reason\":null}}]}}\n\n"
    )
}

/// Upstream that emits one SSE event every `GAP`, then `[DONE]`.
async fn slow_upstream() -> String {
    let handler = || async {
        let events = futures::stream::unfold(0usize, |i| async move {
            if i > CHUNKS {
                return None;
            }
            if i > 0 {
                tokio::time::sleep(GAP).await;
            }
            let body = match i {
                0 => sse(r#""role":"assistant","content":"""#),
                i if i < CHUNKS => sse(&format!(r#""reasoning_content":"t{i} ""#)),
                _ => "data: [DONE]\n\n".to_string(),
            };
            Some((Ok::<_, std::convert::Infallible>(body), i + 1))
        });
        axum::response::Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(events))
            .unwrap()
    };
    let app = axum::Router::new().route("/v1/chat/completions", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });
    format!("http://{addr}/v1")
}

async fn serve_router(upstream: &str) -> (String, String) {
    let db = Database::test().await;
    let metrics_store = MetricsStore::new(1000);
    let router = Arc::new(crate::router::engine::Router::new(metrics_store.clone(), Arc::new(db.clone())));
    let provider = Arc::new(OpenAiProvider::new("slow", Some("slow"), upstream, None));
    router.add_provider(provider.clone()).await;
    router.register_route("slow", vec![provider]).await;

    let session_store = Arc::new(SessionStore::new());
    let state = Arc::new(AppState {
        config: crate::config::AppConfig {
            db: Arc::new(db.clone()),
            router,
            auth_config: crate::auth::nip98::AuthConfig::default(),
            payments_config: None,
            discovery_config: None,
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
        limits_cache: Default::default(),
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

    let app = create_test_app(state).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok() });
    (format!("http://{addr}"), token)
}

#[tokio::test]
async fn first_chunk_arrives_before_the_upstream_finishes() {
    let upstream = slow_upstream().await;
    let (base, token) = serve_router(&upstream).await;

    let started = Instant::now();
    let response = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(token)
        .header("accept-encoding", "gzip, br, zstd")
        .json(&serde_json::json!({
            "model": "slow",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));

    let mut body = response.bytes_stream();
    let mut arrivals = Vec::new();
    while let Some(piece) = body.next().await {
        let piece = piece.unwrap();
        if piece.windows(5).any(|w| w == b"data:") {
            arrivals.push(started.elapsed());
        }
    }
    let total = started.elapsed();

    assert!(arrivals.len() >= CHUNKS, "expected several pieces, got {arrivals:?}");
    let first = arrivals[0];
    assert!(
        first < total.saturating_sub(GAP * 2),
        "first chunk at {first:?} but the stream only ended at {total:?}: the response was buffered ({arrivals:?})"
    );
    assert!(
        arrivals[arrivals.len() - 1] - first >= GAP * 2,
        "chunks were not spread out: {arrivals:?}"
    );
}
