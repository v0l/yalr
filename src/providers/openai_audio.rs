use bytes::Bytes;
use futures::StreamExt;

use super::audio::{
    audio_mime_for_format, SpeechRequest, SpeechResponse, TranscriptionRequest,
    TranscriptionResponse,
};
use super::{OpenAiProvider, ProviderError};

/// Classify an upstream audio error response.
fn http_error(status: reqwest::StatusCode, retry_after: Option<u64>, body: String) -> ProviderError {
    match status.as_u16() {
        429 => ProviderError::RateLimit {
            retry_after_ms: retry_after.map(|s| s * 1000).unwrap_or(30_000),
            message: body,
        },
        401 | 403 => ProviderError::Authentication(body),
        // A backend without the audio route answers 404/405/501. That is a
        // missing capability, not a sick provider, so it must not count as a
        // failure against its health.
        404 | 405 | 501 => ProviderError::Unsupported(body),
        code => ProviderError::ServerError {
            message: body,
            status_code: Some(code),
        },
    }
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

impl OpenAiProvider {
    pub(crate) async fn audio_transcriptions(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResponse, ProviderError> {
        let url = format!("{}{}", self.base_url, request.endpoint());

        let mut part = reqwest::multipart::Part::bytes(request.file.to_vec())
            .file_name(request.file_name.clone());
        if let Some(ct) = &request.content_type {
            part = part
                .mime_str(ct)
                .map_err(|e| ProviderError::Other(Box::new(e)))?;
        }

        let mut form = reqwest::multipart::Form::new()
            .text("model", request.model.clone())
            .part("file", part);
        if let Some(v) = &request.language {
            form = form.text("language", v.clone());
        }
        if let Some(v) = &request.prompt {
            form = form.text("prompt", v.clone());
        }
        if let Some(v) = &request.response_format {
            form = form.text("response_format", v.clone());
        }
        if let Some(v) = request.temperature {
            form = form.text("temperature", v.to_string());
        }
        for granularity in &request.timestamp_granularities {
            form = form.text("timestamp_granularities[]", granularity.clone());
        }

        let mut req = self.http_client.post(&url).multipart(form);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let response = req.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();
        let retry_after = retry_after_secs(response.headers());

        let body = response.bytes().await.map_err(map_reqwest_error)?;
        if !status.is_success() {
            return Err(http_error(
                status,
                retry_after,
                String::from_utf8_lossy(&body).to_string(),
            ));
        }

        Ok(TranscriptionResponse { content_type, body })
    }

    pub(crate) async fn audio_speech(
        &self,
        request: &SpeechRequest,
    ) -> Result<SpeechResponse, ProviderError> {
        let url = format!("{}/audio/speech", self.base_url);

        let mut req = self.http_client.post(&url).json(request);
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
            .map(str::to_string)
            .unwrap_or_else(|| {
                audio_mime_for_format(request.response_format.as_deref()).to_string()
            });

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
            .map(|chunk| chunk.map(Bytes::from).map_err(map_reqwest_error));

        Ok(SpeechResponse {
            content_type,
            stream: Box::pin(stream),
        })
    }
}

fn map_reqwest_error(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout
    } else {
        ProviderError::Other(Box::new(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn transcription_request(translate: bool) -> TranscriptionRequest {
        TranscriptionRequest {
            model: "whisper-1".into(),
            file_name: "clip.wav".into(),
            file: Bytes::from_static(b"RIFF...."),
            content_type: Some("audio/wav".into()),
            language: Some("en".into()),
            prompt: None,
            response_format: Some("json".into()),
            temperature: Some(0.0),
            timestamp_granularities: vec![],
            translate,
        }
    }

    fn speech_request() -> SpeechRequest {
        SpeechRequest {
            model: "tts-1".into(),
            input: "hello".into(),
            voice: Some("alloy".into()),
            response_format: None,
            speed: None,
            instructions: None,
            extra: Default::default(),
        }
    }

    #[tokio::test]
    async fn transcription_posts_multipart_and_passes_body_through() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"text":"hello"}"#, "application/json"),
            )
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let response = provider
            .audio_transcriptions(&transcription_request(false))
            .await
            .unwrap();

        assert_eq!(response.content_type, "application/json");
        assert_eq!(response.text().as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn translate_hits_translations_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/translations"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("hi", "text/plain"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let response = provider
            .audio_transcriptions(&transcription_request(true))
            .await
            .unwrap();
        assert_eq!(&response.body[..], b"hi");
    }

    #[tokio::test]
    async fn transcription_rate_limit_is_classified() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", "7")
                    .set_body_string("slow down"),
            )
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let err = provider
            .audio_transcriptions(&transcription_request(false))
            .await
            .unwrap_err();
        match err {
            ProviderError::RateLimit { retry_after_ms, .. } => {
                assert_eq!(retry_after_ms, 7000)
            }
            other => panic!("expected rate limit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_audio_route_is_unsupported_not_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let err = provider.audio_speech(&speech_request()).await.unwrap_err();
        assert!(matches!(err, ProviderError::Unsupported(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn speech_streams_audio_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1u8, 2, 3], "audio/mpeg"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let response = provider.audio_speech(&speech_request()).await.unwrap();
        assert_eq!(response.content_type, "audio/mpeg");

        let mut collected = Vec::new();
        let mut stream = response.stream;
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(collected, vec![1u8, 2, 3]);
    }

    #[tokio::test]
    async fn speech_error_body_surfaces_as_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("t", None, &server.uri(), Some("key"));
        let err = provider.audio_speech(&speech_request()).await.unwrap_err();
        match err {
            ProviderError::ServerError { status_code, message } => {
                assert_eq!(status_code, Some(500));
                assert_eq!(message, "boom");
            }
            other => panic!("expected server error, got {other:?}"),
        }
    }
}
