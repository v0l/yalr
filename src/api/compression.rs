use axum::Router;
use tower_http::{compression::CompressionLayer, decompression::RequestDecompressionLayer};

pub fn with_compression(app: Router) -> Router {
    app.layer(CompressionLayer::new())
        .layer(RequestDecompressionLayer::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::post};
    use std::io::{Read, Write};
    use tower::util::ServiceExt;

    fn app() -> Router {
        with_compression(Router::new().route("/echo", post(|body: String| async move { body })))
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    async fn read(response: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()
    }

    #[tokio::test]
    async fn gzipped_request_bodies_are_decoded() {
        let payload = r#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#.repeat(40);
        let response = app()
            .oneshot(
                Request::post("/echo")
                    .header("content-encoding", "gzip")
                    .body(Body::from(gzip(payload.as_bytes())))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(read(response).await, payload.as_bytes());
    }

    #[tokio::test]
    async fn plain_request_bodies_still_work() {
        let response = app()
            .oneshot(Request::post("/echo").body(Body::from("plain")).unwrap())
            .await
            .unwrap();
        assert_eq!(read(response).await, b"plain");
    }

    #[tokio::test]
    async fn responses_are_compressed_for_clients_that_accept_it() {
        let payload = "the same words again ".repeat(200);
        let response = app()
            .oneshot(
                Request::post("/echo")
                    .header("accept-encoding", "gzip")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()["content-encoding"], "gzip");
        let packed = read(response).await;
        assert!(packed.len() < payload.len() / 4);
        let mut unpacked = String::new();
        flate2::read::GzDecoder::new(&packed[..]).read_to_string(&mut unpacked).unwrap();
        assert_eq!(unpacked, payload);
    }

    #[tokio::test]
    async fn responses_stay_plain_without_accept_encoding() {
        let payload = "the same words again ".repeat(200);
        let response = app()
            .oneshot(Request::post("/echo").body(Body::from(payload.clone())).unwrap())
            .await
            .unwrap();
        assert!(response.headers().get("content-encoding").is_none());
        assert_eq!(read(response).await, payload.as_bytes());
    }

    #[tokio::test]
    async fn event_streams_are_never_compressed() {
        let stream_app = with_compression(Router::new().route(
            "/sse",
            axum::routing::get(|| async {
                (
                    [("content-type", "text/event-stream")],
                    "data: hello\n\n".repeat(200),
                )
            }),
        ));
        let response = stream_app
            .oneshot(Request::get("/sse").header("accept-encoding", "gzip").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.headers().get("content-encoding").is_none());
    }

    #[tokio::test]
    async fn websocket_upgrade_survives_a_gzip_accepting_client() {
        use axum::extract::ws::WebSocketUpgrade;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let app = with_compression(Router::new().route(
            "/ws",
            axum::routing::get(|ws: WebSocketUpgrade| async move { ws.on_upgrade(|_socket| async {}) }),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });

        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket
            .write_all(
                format!(
                    "GET /ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                     Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
                     Accept-Encoding: gzip, deflate, br, zstd\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = vec![0u8; 1024];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), socket.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 101"), "{head}");
        assert!(head.contains("sec-websocket-accept"), "{head}");
        assert!(!head.contains("content-encoding"), "{head}");
    }
}
