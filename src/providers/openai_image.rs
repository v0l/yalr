use futures::StreamExt;

use super::image::{ImageGenerationRequest, ImageResponse, ImageUploadRequest};
use super::openai_audio::{http_error, map_reqwest_error, retry_after_secs};
use super::{OpenAiProvider, ProviderError};

impl OpenAiProvider {
    pub(crate) async fn image_generations(
        &self,
        request: &ImageGenerationRequest,
    ) -> Result<ImageResponse, ProviderError> {
        let url = format!("{}/images/generations", self.base_url);
        self.send_image_request(self.http_client.post(&url).json(request))
            .await
    }

    pub(crate) async fn image_uploads(
        &self,
        request: &ImageUploadRequest,
    ) -> Result<ImageResponse, ProviderError> {
        let url = format!("{}{}", self.base_url, request.endpoint());

        let mut form = reqwest::multipart::Form::new().text("model", request.model.clone());
        for (name, value) in &request.fields {
            form = form.text(name.clone(), value.clone());
        }
        for file in &request.files {
            let mut part = reqwest::multipart::Part::bytes(file.data.to_vec())
                .file_name(file.file_name.clone());
            if let Some(ct) = &file.content_type {
                part = part
                    .mime_str(ct)
                    .map_err(|e| ProviderError::Other(Box::new(e)))?;
            }
            form = form.part(file.field.clone(), part);
        }

        self.send_image_request(self.http_client.post(&url).multipart(form))
            .await
    }

    async fn send_image_request(
        &self,
        mut req: reqwest::RequestBuilder,
    ) -> Result<ImageResponse, ProviderError> {
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let response = req.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        let retry_after = retry_after_secs(response.headers());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();

        if !status.is_success() {
            let body = response.bytes().await.unwrap_or_default();
            return Err(http_error(
                status,
                retry_after,
                String::from_utf8_lossy(&body).to_string(),
            ));
        }

        let stream = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(map_reqwest_error));

        Ok(ImageResponse {
            content_type,
            stream: Box::pin(stream),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::image::{ImageFile, ImageUploadKind};
    use bytes::Bytes;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const IMAGES: &str = r#"{"created":1,"data":[{"b64_json":"aGk="}]}"#;

    fn generation() -> ImageGenerationRequest {
        serde_json::from_str(r#"{"model":"gpt-image-1","prompt":"a cat","size":"1024x1024"}"#)
            .unwrap()
    }

    fn upload(kind: ImageUploadKind) -> ImageUploadRequest {
        ImageUploadRequest {
            kind,
            model: "gpt-image-1".into(),
            files: vec![ImageFile {
                field: "image[]".into(),
                file_name: "cat.png".into(),
                content_type: Some("image/png".into()),
                data: Bytes::from_static(b"PNGDATA"),
            }],
            fields: vec![("prompt".into(), "add a hat".into())],
        }
    }

    async fn collect(response: ImageResponse) -> Vec<u8> {
        let mut out = Vec::new();
        let mut stream = response.stream;
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        out
    }

    #[tokio::test]
    async fn generation_posts_json_and_passes_body_through() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .and(header("authorization", "Bearer key"))
            .and(body_string_contains(r#""size":"1024x1024""#))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let response = provider.image_generations(&generation()).await.unwrap();
        assert_eq!(response.content_type, "application/json");
        assert_eq!(collect(response).await, IMAGES.as_bytes());
    }

    #[tokio::test]
    async fn edit_posts_multipart_with_files_and_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/edits"))
            .and(body_string_contains("name=\"image[]\"; filename=\"cat.png\""))
            .and(body_string_contains("PNGDATA"))
            .and(body_string_contains("add a hat"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let response = provider
            .image_uploads(&upload(ImageUploadKind::Edit))
            .await
            .unwrap();
        assert_eq!(collect(response).await, IMAGES.as_bytes());
    }

    #[tokio::test]
    async fn variation_hits_variations_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/variations"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(IMAGES, "application/json"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        assert!(provider
            .image_uploads(&upload(ImageUploadKind::Variation))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn missing_image_route_is_unsupported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let err = provider.image_generations(&generation()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn content_policy_rejection_is_a_client_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_string("safety system"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let err = provider.image_generations(&generation()).await.unwrap_err();
        assert!(err.is_client_error(), "got {err:?}");
        assert!(!err.is_transient());
    }
}
