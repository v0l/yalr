use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureAttribution {
    #[default]
    Provider,
    Request,
    Unknown,
}

impl FailureAttribution {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureAttribution::Provider => "provider",
            FailureAttribution::Request => "request",
            FailureAttribution::Unknown => "unknown",
        }
    }

    pub fn weight(self) -> f64 {
        match self {
            FailureAttribution::Provider => 1.0,
            FailureAttribution::Request => 0.0,
            FailureAttribution::Unknown => 0.5,
        }
    }
}

const REQUEST_ERROR_PATTERNS: &[(&str, &[&str])] = &[
    (
        "context_length",
        &[
            "maximum context length",
            "context_length_exceeded",
            "context length",
            "context window",
            "prompt is too long",
            "input is too long",
            "too many tokens",
            "exceeds the maximum",
            "exceeds the model",
            "max_model_len",
        ],
    ),
    (
        "media",
        &[
            "invalid image",
            "unsupported image",
            "failed to decode image",
            "image too large",
            "too many images",
            "invalid media",
            "unsupported media",
            "media limit",
            "invalid base64",
            "could not process image",
        ],
    ),
    (
        "parameter",
        &[
            "unsupported parameter",
            "unknown parameter",
            "unrecognized request argument",
            "invalid parameter",
            "unsupported value",
            "invalid_request_error",
            "extra inputs are not permitted",
            "not supported with this model",
        ],
    ),
    ("watchdog", &["watchdog"]),
    (
        "client_disconnect",
        &["client disconnected", "client closed request", "request cancelled", "request canceled"],
    ),
];

const CONNECTION_PATTERNS: &[&str] = &[
    "connection refused",
    "connection reset",
    "connection closed before",
    "error sending request",
    "tcp connect error",
    "dns error",
    "failed to lookup address",
    "name or service not known",
    "tls handshake",
    "certificate",
    "network is unreachable",
    "no route to host",
];

pub fn request_error_class(message: &str) -> Option<&'static str> {
    let lowered = message.to_lowercase();
    REQUEST_ERROR_PATTERNS
        .iter()
        .find(|(_, needles)| needles.iter().any(|n| lowered.contains(n)))
        .map(|(class, _)| *class)
}

pub fn is_connection_failure_text(message: &str) -> bool {
    let lowered = message.to_lowercase();
    CONNECTION_PATTERNS.iter().any(|n| lowered.contains(n))
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AttemptInfo {
    pub prompt_tokens: Option<u32>,
    pub waited_ms: Option<u64>,
    pub progressed: bool,
}

const FIRST_TOKEN_BASE_MS: f64 = 2_000.0;
const FIRST_TOKEN_MARGIN: f64 = 2.0;

pub fn expected_first_token_ms(prompt_tokens: u32, prefill_tokens_per_s: f64) -> f64 {
    FIRST_TOKEN_BASE_MS + FIRST_TOKEN_MARGIN * prompt_tokens as f64 / prefill_tokens_per_s * 1000.0
}

pub fn attribute_timeout(info: &AttemptInfo, prefill_tokens_per_s: Option<f64>) -> FailureAttribution {
    if info.progressed {
        return FailureAttribution::Request;
    }
    match (info.prompt_tokens, info.waited_ms, prefill_tokens_per_s) {
        (Some(prompt), Some(waited), Some(rate)) if rate > 0.0 => {
            if waited as f64 > expected_first_token_ms(prompt, rate) {
                FailureAttribution::Provider
            } else {
                FailureAttribution::Request
            }
        }
        _ => FailureAttribution::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_token_timeout_is_scaled_by_prompt_size() {
        let wait = |prompt| AttemptInfo { prompt_tokens: Some(prompt), waited_ms: Some(60_000), progressed: false };
        assert_eq!(attribute_timeout(&wait(1_000), Some(1_000.0)), FailureAttribution::Provider);
        assert_eq!(attribute_timeout(&wait(200_000), Some(1_000.0)), FailureAttribution::Request);
    }

    #[test]
    fn a_timeout_without_a_measured_rate_is_unknown() {
        let info = AttemptInfo { prompt_tokens: Some(10), waited_ms: Some(60_000), progressed: false };
        assert_eq!(attribute_timeout(&info, None), FailureAttribution::Unknown);
    }

    #[test]
    fn a_total_timeout_on_a_stream_that_made_progress_is_a_request_fault() {
        let info = AttemptInfo { prompt_tokens: Some(10), waited_ms: Some(600_000), progressed: true };
        assert_eq!(attribute_timeout(&info, Some(1_000.0)), FailureAttribution::Request);
    }

    #[test]
    fn request_patterns_cover_the_known_classes() {
        assert_eq!(request_error_class("This model's maximum context length is 8192"), Some("context_length"));
        assert_eq!(request_error_class("Invalid image data"), Some("media"));
        assert_eq!(request_error_class("Unsupported parameter: foo"), Some("parameter"));
        assert_eq!(request_error_class("upstream exploded"), None);
    }
}
