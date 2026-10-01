use std::collections::HashMap;

use bytes::Bytes;
use futures::stream::BoxStream;

use super::ProviderError;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ImageGenerationRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(flatten, default)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageUploadKind {
    Edit,
    Variation,
}

#[derive(Debug, Clone)]
pub struct ImageFile {
    pub field: String,
    pub file_name: String,
    pub content_type: Option<String>,
    pub data: Bytes,
}

#[derive(Debug, Clone)]
pub struct ImageUploadRequest {
    pub kind: ImageUploadKind,
    pub model: String,
    pub files: Vec<ImageFile>,
    pub fields: Vec<(String, String)>,
}

impl ImageUploadRequest {
    pub fn endpoint(&self) -> &'static str {
        match self.kind {
            ImageUploadKind::Edit => "/images/edits",
            ImageUploadKind::Variation => "/images/variations",
        }
    }

    pub fn operation(&self) -> &'static str {
        match self.kind {
            ImageUploadKind::Edit => "images.edits",
            ImageUploadKind::Variation => "images.variations",
        }
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub struct ImageResponse {
    pub content_type: String,
    pub stream: BoxStream<'static, Result<Bytes, ProviderError>>,
}

impl std::fmt::Debug for ImageResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageResponse")
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_request_keeps_unknown_fields_and_omits_absent_ones() {
        let parsed: ImageGenerationRequest = serde_json::from_str(
            r#"{"model":"gpt-image-1","prompt":"a cat","background":"transparent"}"#,
        )
        .unwrap();
        assert_eq!(parsed.extra.get("background").unwrap(), "transparent");

        let wire = serde_json::to_value(&parsed).unwrap();
        assert_eq!(wire.get("background").unwrap(), "transparent");
        assert!(wire.get("n").is_none());
        assert!(wire.get("size").is_none());
    }

    #[test]
    fn upload_endpoint_follows_kind() {
        let mut request = ImageUploadRequest {
            kind: ImageUploadKind::Edit,
            model: "gpt-image-1".into(),
            files: vec![],
            fields: vec![("prompt".into(), "hat".into())],
        };
        assert_eq!(request.endpoint(), "/images/edits");
        assert_eq!(request.field("prompt"), Some("hat"));
        request.kind = ImageUploadKind::Variation;
        assert_eq!(request.endpoint(), "/images/variations");
    }
}
