use crate::api::shared::ApiError;
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};

fn is_plain_text(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/plain"))
}

pub async fn request_too_large_json(
    State(limit_bytes): State<usize>,
    req: Request,
    next: Next,
) -> Response {
    let response = next.run(req).await;
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE && is_plain_text(&response) {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(ApiError::request_too_large(limit_bytes)),
        )
            .into_response();
    }
    response
}
