use std::sync::Arc;

use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::AnthropicOAuthProvider;
use crate::db::{Database, Provider as ProviderRecord, ProviderType};
use crate::providers::{ChatRequest, Provider, ProviderError};

async fn provider(base_url: &str) -> AnthropicOAuthProvider {
    let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
    let record = ProviderRecord {
        id: 1,
        name: "claude".into(),
        slug: "claude".into(),
        base_url: base_url.into(),
        api_key: None,
        provider_type: ProviderType::AnthropicOauth,
        oauth_access_token: Some("token".into()),
        oauth_refresh_token: Some("refresh".into()),
        oauth_expires_at: Some(i64::MAX),
        oauth_account_id: None,
        client_version: None,
        created_at: String::new(),
        updated_at: String::new(),
    };
    AnthropicOAuthProvider::new(&record, db)
}

fn request() -> ChatRequest {
    serde_json::from_str(r#"{"model":"claude-opus-5-5","stream":true,"messages":[{"role":"user","content":"hi"}]}"#)
        .unwrap()
}

fn sse(events: &[&str]) -> String {
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

async fn serve(body: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(&server)
        .await;
    server
}

async fn collect(server: &MockServer) -> (String, Vec<String>, Option<ProviderError>) {
    let provider = provider(&server.uri()).await;
    let mut stream = provider.chat_completions_stream(&request()).unwrap();
    let (mut text, mut finish, mut error) = (String::new(), Vec::new(), None);
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                for choice in chunk.choices {
                    text.push_str(choice.delta.content.as_deref().unwrap_or(""));
                    finish.extend(choice.finish_reason);
                }
            }
            Err(e) => error = Some(e),
        }
    }
    (text, finish, error)
}

#[tokio::test]
async fn max_tokens_stop_reports_length() {
    let server = serve(
        sse(&[
            r#"{"type":"message_start","message":{"id":"m","usage":{"input_tokens":3}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"cut"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":1}}"#,
        ])
        .into_bytes(),
    )
    .await;
    let (text, finish, error) = collect(&server).await;
    assert_eq!(text, "cut");
    assert_eq!(finish, vec!["length"]);
    assert!(error.is_none());
}

#[tokio::test]
async fn overloaded_error_event_surfaces_as_transient_error() {
    let server = serve(
        sse(&[
            r#"{"type":"message_start","message":{"id":"m"}}"#,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ])
        .into_bytes(),
    )
    .await;
    let (_, finish, error) = collect(&server).await;
    assert!(finish.is_empty());
    let error = error.expect("error event becomes an error");
    assert!(error.is_transient());
    assert!(error.to_string().contains("Overloaded"));
}

#[test]
fn multibyte_text_survives_a_split_line() {
    let body = sse(&[
        r#"{"type":"message_start","message":{"id":"m"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"日本🎉"}}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}"#,
    ])
    .into_bytes();
    let split = body.windows(3).position(|w| w == "本".as_bytes()).unwrap() + 1;
    let (head, tail) = body.split_at(split);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (head, tail) = (head.to_vec(), tail.to_vec());
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut scratch = vec![0u8; 65536];
            let _ = socket.read(&mut scratch).await;
            let header = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
            socket.write_all(header.as_bytes()).await.unwrap();
            for part in [head, tail] {
                socket.write_all(format!("{:x}\r\n", part.len()).as_bytes()).await.unwrap();
                socket.write_all(&part).await.unwrap();
                socket.write_all(b"\r\n").await.unwrap();
                socket.flush().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            socket.write_all(b"0\r\n\r\n").await.unwrap();
        });
        let provider = provider(&format!("http://{addr}")).await;
        let mut stream = provider.chat_completions_stream(&request()).unwrap();
        let mut text = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            for choice in chunk.choices {
                text.push_str(choice.delta.content.as_deref().unwrap_or(""));
            }
        }
        assert_eq!(text, "日本🎉");
    });
}
