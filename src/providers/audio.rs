use bytes::Bytes;
use futures::stream::BoxStream;

use super::ProviderError;

/// Speech-to-text request for `/v1/audio/transcriptions` and
/// `/v1/audio/translations`.
///
/// The audio is held in memory: upstreams want a multipart body with a known
/// length, and OpenAI caps uploads at 25 MB anyway.
#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    pub model: String,
    pub file_name: String,
    pub file: Bytes,
    pub content_type: Option<String>,
    pub language: Option<String>,
    pub prompt: Option<String>,
    pub response_format: Option<String>,
    pub temperature: Option<f32>,
    pub timestamp_granularities: Vec<String>,
    /// Target `/v1/audio/translations` (always English output) instead of
    /// `/v1/audio/transcriptions`.
    pub translate: bool,
}

impl TranscriptionRequest {
    pub fn endpoint(&self) -> &'static str {
        if self.translate {
            "/audio/translations"
        } else {
            "/audio/transcriptions"
        }
    }
}

/// Upstream transcription body, passed through verbatim.
///
/// `response_format` may be `json`, `verbose_json`, `text`, `srt` or `vtt`, so
/// parsing here would only throw away shapes we do not control.
#[derive(Debug, Clone)]
pub struct TranscriptionResponse {
    pub content_type: String,
    pub body: Bytes,
}

impl TranscriptionResponse {
    /// The transcript text, when the upstream returned a JSON body.
    pub fn text(&self) -> Option<String> {
        #[derive(serde::Deserialize)]
        struct TextOnly {
            text: String,
        }
        serde_json::from_slice::<TextOnly>(&self.body)
            .ok()
            .map(|t| t.text)
    }
}

/// Text-to-speech request for `/v1/audio/speech`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SpeechRequest {
    pub model: String,
    pub input: String,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Catch-all so backend-specific knobs (Kokoro's `lang_code`, speaker ids)
    /// survive the round-trip.
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

/// Generated audio, streamed as it arrives.
pub struct SpeechResponse {
    pub content_type: String,
    pub stream: BoxStream<'static, Result<Bytes, ProviderError>>,
}

impl std::fmt::Debug for SpeechResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeechResponse")
            .field("content_type", &self.content_type)
            .finish_non_exhaustive()
    }
}

/// Default MIME type for a `response_format` value, used when the upstream
/// omits `Content-Type`.
pub fn audio_mime_for_format(format: Option<&str>) -> &'static str {
    match format.unwrap_or("mp3") {
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        _ => "audio/mpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(translate: bool) -> TranscriptionRequest {
        TranscriptionRequest {
            model: "whisper-1".into(),
            file_name: "a.wav".into(),
            file: Bytes::from_static(b"riff"),
            content_type: None,
            language: None,
            prompt: None,
            response_format: None,
            temperature: None,
            timestamp_granularities: vec![],
            translate,
        }
    }

    #[test]
    fn endpoint_follows_translate_flag() {
        assert_eq!(req(false).endpoint(), "/audio/transcriptions");
        assert_eq!(req(true).endpoint(), "/audio/translations");
    }

    #[test]
    fn text_extracted_only_from_json() {
        let json = TranscriptionResponse {
            content_type: "application/json".into(),
            body: Bytes::from_static(br#"{"text":"hello there"}"#),
        };
        assert_eq!(json.text().as_deref(), Some("hello there"));

        let srt = TranscriptionResponse {
            content_type: "text/plain".into(),
            body: Bytes::from_static(b"1\n00:00:00,000 --> 00:00:01,000\nhi\n"),
        };
        assert_eq!(srt.text(), None);
    }

    #[test]
    fn speech_request_keeps_unknown_fields() {
        let parsed: SpeechRequest = serde_json::from_str(
            r#"{"model":"tts-1","input":"hi","voice":"alloy","lang_code":"en"}"#,
        )
        .unwrap();
        assert_eq!(parsed.extra.get("lang_code").unwrap(), "en");
        let wire = serde_json::to_value(&parsed).unwrap();
        assert_eq!(wire.get("lang_code").unwrap(), "en");
        assert!(wire.get("speed").is_none());
    }

    #[test]
    fn mime_defaults_to_mp3() {
        assert_eq!(audio_mime_for_format(None), "audio/mpeg");
        assert_eq!(audio_mime_for_format(Some("wav")), "audio/wav");
        assert_eq!(audio_mime_for_format(Some("weird")), "audio/mpeg");
    }
}
