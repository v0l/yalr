use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Envelope {
    error: Detail,
}

#[derive(Deserialize)]
struct Detail {
    retry_after_seconds: Option<u64>,
}

pub async fn add_header(response: Response) -> Response {
    if !matches!(response.status(), StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE) {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 1 << 20).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let seconds = serde_json::from_slice::<Envelope>(&bytes).ok().and_then(|e| e.error.retry_after_seconds);
    if let Some(value) = seconds.and_then(|s| HeaderValue::from_str(&s.to_string()).ok()) {
        parts.headers.insert(header::RETRY_AFTER, value);
    }
    Response::from_parts(parts, Body::from(bytes))
}
