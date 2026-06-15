//! Helpers for extracting `QuotaSnapshot`s from upstream rate-limit response
//! headers. Used by the OAuth subscription providers (Claude Max, ChatGPT),
//! whose usage quota is reported via headers on each API response rather than a
//! dedicated balance endpoint.

use reqwest::header::HeaderMap;

use crate::providers::provider_trait::QuotaSnapshot;

fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn header_i64(headers: &HeaderMap, name: &str) -> Option<i64> {
    header_str(headers, name).and_then(|s| s.parse::<i64>().ok())
}

/// Parse a reset value into epoch milliseconds.
///
/// Accepts RFC 3339 timestamps (Anthropic), epoch seconds, or relative
/// durations like `"6m0s"` / `"1s"` (OpenAI).
fn parse_reset_to_epoch_ms(value: &str) -> Option<i64> {
    // RFC 3339 / ISO 8601 timestamp.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(dt.timestamp_millis());
    }
    // Plain numeric: treat as epoch seconds (Unix reset timestamp).
    if let Ok(secs) = value.parse::<i64>() {
        // Heuristic: values that look like epoch seconds (> year 2001).
        if secs > 1_000_000_000 {
            return Some(secs * 1000);
        }
        // Otherwise treat as a relative offset in seconds.
        return Some(crate::oauth::now_ms() + secs * 1000);
    }
    // Duration form like "6m0s", "1s", "1m30s", "500ms".
    if let Some(ms) = parse_duration_ms(value) {
        return Some(crate::oauth::now_ms() + ms);
    }
    None
}

/// Parse a Go-style duration string (e.g. `"6m0s"`, `"1s"`, `"500ms"`) to ms.
fn parse_duration_ms(value: &str) -> Option<i64> {
    let mut total: i64 = 0;
    let mut num = String::new();
    let mut chars = value.chars().peekable();
    let mut matched = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            chars.next();
        } else {
            // Read the unit (ms, s, m, h).
            let mut unit = String::new();
            while let Some(&u) = chars.peek() {
                if u.is_ascii_alphabetic() {
                    unit.push(u);
                    chars.next();
                } else {
                    break;
                }
            }
            let n: f64 = num.parse().ok()?;
            let mult = match unit.as_str() {
                "ms" => 1.0,
                "s" => 1000.0,
                "m" => 60_000.0,
                "h" => 3_600_000.0,
                _ => return None,
            };
            total += (n * mult) as i64;
            num.clear();
            matched = true;
        }
    }
    if matched { Some(total) } else { None }
}

fn used_pct(remaining: Option<i64>, limit: Option<i64>) -> Option<f32> {
    match (remaining, limit) {
        (Some(r), Some(l)) if l > 0 => {
            let used = (l - r).max(0) as f32;
            Some((used / l as f32) * 100.0)
        }
        _ => None,
    }
}

/// Extract a quota snapshot from Anthropic rate-limit headers.
///
/// Prefers the unified (subscription) limit, falling back to the standard
/// token/request limits. Returns `None` if no relevant headers are present.
pub fn anthropic_quota_from_headers(headers: &HeaderMap) -> Option<QuotaSnapshot> {
    // Unified limit used for Claude subscription OAuth.
    let unified_status = header_str(headers, "anthropic-ratelimit-unified-status");
    let unified_remaining = header_i64(headers, "anthropic-ratelimit-unified-remaining");
    let unified_limit = header_i64(headers, "anthropic-ratelimit-unified-limit");
    let unified_reset = header_str(headers, "anthropic-ratelimit-unified-reset");

    if unified_status.is_some() || unified_remaining.is_some() || unified_reset.is_some() {
        return Some(QuotaSnapshot {
            remaining: unified_remaining,
            limit: unified_limit,
            used_pct: used_pct(unified_remaining, unified_limit),
            resets_at: unified_reset.as_deref().and_then(parse_reset_to_epoch_ms),
            window: Some("unified".to_string()),
            status: unified_status,
        });
    }

    // Standard token-based limit.
    let tok_remaining = header_i64(headers, "anthropic-ratelimit-tokens-remaining");
    let tok_limit = header_i64(headers, "anthropic-ratelimit-tokens-limit");
    let tok_reset = header_str(headers, "anthropic-ratelimit-tokens-reset");
    if tok_remaining.is_some() || tok_limit.is_some() {
        return Some(QuotaSnapshot {
            remaining: tok_remaining,
            limit: tok_limit,
            used_pct: used_pct(tok_remaining, tok_limit),
            resets_at: tok_reset.as_deref().and_then(parse_reset_to_epoch_ms),
            window: Some("tokens".to_string()),
            status: None,
        });
    }

    None
}

/// Extract a quota snapshot from OpenAI-style `x-ratelimit-*` headers.
pub fn openai_quota_from_headers(headers: &HeaderMap) -> Option<QuotaSnapshot> {
    // Prefer token-based limit, fall back to request-based.
    let tok_remaining = header_i64(headers, "x-ratelimit-remaining-tokens");
    let tok_limit = header_i64(headers, "x-ratelimit-limit-tokens");
    let tok_reset = header_str(headers, "x-ratelimit-reset-tokens");
    if tok_remaining.is_some() || tok_limit.is_some() {
        return Some(QuotaSnapshot {
            remaining: tok_remaining,
            limit: tok_limit,
            used_pct: used_pct(tok_remaining, tok_limit),
            resets_at: tok_reset.as_deref().and_then(parse_reset_to_epoch_ms),
            window: Some("tokens".to_string()),
            status: None,
        });
    }

    let req_remaining = header_i64(headers, "x-ratelimit-remaining-requests");
    let req_limit = header_i64(headers, "x-ratelimit-limit-requests");
    let req_reset = header_str(headers, "x-ratelimit-reset-requests");
    if req_remaining.is_some() || req_limit.is_some() {
        return Some(QuotaSnapshot {
            remaining: req_remaining,
            limit: req_limit,
            used_pct: used_pct(req_remaining, req_limit),
            resets_at: req_reset.as_deref().and_then(parse_reset_to_epoch_ms),
            window: Some("requests".to_string()),
            status: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn hm(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn test_parse_duration_ms() {
        assert_eq!(parse_duration_ms("1s"), Some(1000));
        assert_eq!(parse_duration_ms("6m0s"), Some(360_000));
        assert_eq!(parse_duration_ms("1m30s"), Some(90_000));
        assert_eq!(parse_duration_ms("500ms"), Some(500));
        assert_eq!(parse_duration_ms("2h"), Some(7_200_000));
        assert_eq!(parse_duration_ms("abc"), None);
    }

    #[test]
    fn test_parse_reset_rfc3339() {
        let ms = parse_reset_to_epoch_ms("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(ms, 1_767_225_600_000);
    }

    #[test]
    fn test_parse_reset_epoch_seconds() {
        let ms = parse_reset_to_epoch_ms("1767225600").unwrap();
        assert_eq!(ms, 1_767_225_600_000);
    }

    #[test]
    fn test_used_pct() {
        assert_eq!(used_pct(Some(25), Some(100)), Some(75.0));
        assert_eq!(used_pct(Some(100), Some(100)), Some(0.0));
        assert_eq!(used_pct(None, Some(100)), None);
        assert_eq!(used_pct(Some(5), Some(0)), None);
    }

    #[test]
    fn test_anthropic_unified() {
        let h = hm(&[
            ("anthropic-ratelimit-unified-status", "allowed_warning"),
            ("anthropic-ratelimit-unified-remaining", "20"),
            ("anthropic-ratelimit-unified-limit", "100"),
            ("anthropic-ratelimit-unified-reset", "2026-01-01T00:00:00Z"),
        ]);
        let q = anthropic_quota_from_headers(&h).unwrap();
        assert_eq!(q.remaining, Some(20));
        assert_eq!(q.limit, Some(100));
        assert_eq!(q.used_pct, Some(80.0));
        assert_eq!(q.status.as_deref(), Some("allowed_warning"));
        assert_eq!(q.window.as_deref(), Some("unified"));
        assert_eq!(q.resets_at, Some(1_767_225_600_000));
    }

    #[test]
    fn test_anthropic_tokens_fallback() {
        let h = hm(&[
            ("anthropic-ratelimit-tokens-remaining", "5000"),
            ("anthropic-ratelimit-tokens-limit", "20000"),
        ]);
        let q = anthropic_quota_from_headers(&h).unwrap();
        assert_eq!(q.remaining, Some(5000));
        assert_eq!(q.window.as_deref(), Some("tokens"));
        assert_eq!(q.used_pct, Some(75.0));
    }

    #[test]
    fn test_anthropic_none() {
        let h = hm(&[("content-type", "application/json")]);
        assert!(anthropic_quota_from_headers(&h).is_none());
    }

    #[test]
    fn test_openai_tokens() {
        let h = hm(&[
            ("x-ratelimit-remaining-tokens", "8000"),
            ("x-ratelimit-limit-tokens", "10000"),
            ("x-ratelimit-reset-tokens", "6m0s"),
        ]);
        let q = openai_quota_from_headers(&h).unwrap();
        assert_eq!(q.remaining, Some(8000));
        assert_eq!(q.limit, Some(10000));
        assert_eq!(q.used_pct, Some(20.0));
        assert_eq!(q.window.as_deref(), Some("tokens"));
        assert!(q.resets_at.is_some());
    }

    #[test]
    fn test_openai_requests_fallback() {
        let h = hm(&[
            ("x-ratelimit-remaining-requests", "40"),
            ("x-ratelimit-limit-requests", "60"),
        ]);
        let q = openai_quota_from_headers(&h).unwrap();
        assert_eq!(q.remaining, Some(40));
        assert_eq!(q.window.as_deref(), Some("requests"));
    }
}
