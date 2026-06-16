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

fn header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
    header_str(headers, name).and_then(|s| s.parse::<f64>().ok())
}

/// Returns true if a quota window is effectively exhausted (the request that
/// produced it was, or the next would be, blocked).
pub fn quota_exhausted(q: &QuotaSnapshot) -> bool {
    matches!(
        q.status.as_deref(),
        Some("rejected") | Some("exceeded") | Some("rate_limited")
    ) || q.used_pct.is_some_and(|p| p >= 100.0)
        || matches!((q.remaining, q.limit), (Some(r), Some(l)) if l > 0 && r <= 0)
}

/// Severity score used to rank quota windows by how close they are to blocking.
/// Exhausted windows always rank highest.
fn quota_severity(q: &QuotaSnapshot) -> f32 {
    if quota_exhausted(q) {
        return f32::INFINITY;
    }
    q.used_pct.or_else(|| used_pct(q.remaining, q.limit)).unwrap_or(0.0)
}

/// Pick the most-consumed quota window — the one most likely to throttle next.
/// This is what the provider card surfaces ("first to stop").
pub fn worst_quota(quotas: &[QuotaSnapshot]) -> Option<QuotaSnapshot> {
    quotas.iter().cloned().max_by(|a, b| {
        quota_severity(a)
            .partial_cmp(&quota_severity(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
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

/// Per-window unified limits exposed for Claude subscription (OAuth) tokens.
/// Each window reports `utilization` as a float 0.0–1.0 plus a status/reset.
const ANTHROPIC_UNIFIED_WINDOWS: &[&str] = &["5h", "7d", "7d_sonnet", "7d_opus"];

/// Extract all quota windows from Anthropic rate-limit headers.
///
/// Anthropic enforces several independent windows simultaneously:
/// - Subscription OAuth: per-window unified limits (`5h`, `7d`, …).
/// - API key: separate requests / tokens / input-tokens / output-tokens limits.
///
/// Returns every window present so callers can show them all and pick the
/// most-consumed one for compact display. Empty if no relevant headers exist.
pub fn anthropic_quotas_from_headers(headers: &HeaderMap) -> Vec<QuotaSnapshot> {
    let mut out = Vec::new();

    // Per-window unified (subscription) limits. `utilization` is a 0.0–1.0 float.
    for win in ANTHROPIC_UNIFIED_WINDOWS {
        let util = header_f64(headers, &format!("anthropic-ratelimit-unified-{win}-utilization"));
        let status = header_str(headers, &format!("anthropic-ratelimit-unified-{win}-status"));
        let reset = header_str(headers, &format!("anthropic-ratelimit-unified-{win}-reset"));
        if util.is_some() || status.is_some() || reset.is_some() {
            out.push(QuotaSnapshot {
                remaining: None,
                limit: None,
                used_pct: util.map(|u| (u * 100.0) as f32),
                resets_at: reset.as_deref().and_then(parse_reset_to_epoch_ms),
                window: Some((*win).to_string()),
                status,
            });
        }
    }

    // Legacy single unified limit (no per-window suffix).
    if out.is_empty() {
        let unified_status = header_str(headers, "anthropic-ratelimit-unified-status");
        let unified_remaining = header_i64(headers, "anthropic-ratelimit-unified-remaining");
        let unified_limit = header_i64(headers, "anthropic-ratelimit-unified-limit");
        let unified_reset = header_str(headers, "anthropic-ratelimit-unified-reset");
        if unified_status.is_some() || unified_remaining.is_some() || unified_reset.is_some() {
            out.push(QuotaSnapshot {
                remaining: unified_remaining,
                limit: unified_limit,
                used_pct: used_pct(unified_remaining, unified_limit),
                resets_at: unified_reset.as_deref().and_then(parse_reset_to_epoch_ms),
                window: Some("unified".to_string()),
                status: unified_status,
            });
        }
    }

    // Standard per-resource limits (API-key based).
    for (win, prefix) in [
        ("requests", "anthropic-ratelimit-requests"),
        ("tokens", "anthropic-ratelimit-tokens"),
        ("input tokens", "anthropic-ratelimit-input-tokens"),
        ("output tokens", "anthropic-ratelimit-output-tokens"),
    ] {
        let remaining = header_i64(headers, &format!("{prefix}-remaining"));
        let limit = header_i64(headers, &format!("{prefix}-limit"));
        let reset = header_str(headers, &format!("{prefix}-reset"));
        if remaining.is_some() || limit.is_some() {
            out.push(QuotaSnapshot {
                remaining,
                limit,
                used_pct: used_pct(remaining, limit),
                resets_at: reset.as_deref().and_then(parse_reset_to_epoch_ms),
                window: Some(win.to_string()),
                status: None,
            });
        }
    }

    out
}

/// Extract all quota windows from OpenAI-style `x-ratelimit-*` headers
/// (both token- and request-based limits when present).
pub fn openai_quotas_from_headers(headers: &HeaderMap) -> Vec<QuotaSnapshot> {
    let mut out = Vec::new();
    for (win, rem, lim, rst) in [
        (
            "tokens",
            "x-ratelimit-remaining-tokens",
            "x-ratelimit-limit-tokens",
            "x-ratelimit-reset-tokens",
        ),
        (
            "requests",
            "x-ratelimit-remaining-requests",
            "x-ratelimit-limit-requests",
            "x-ratelimit-reset-requests",
        ),
    ] {
        let remaining = header_i64(headers, rem);
        let limit = header_i64(headers, lim);
        let reset = header_str(headers, rst);
        if remaining.is_some() || limit.is_some() {
            out.push(QuotaSnapshot {
                remaining,
                limit,
                used_pct: used_pct(remaining, limit),
                resets_at: reset.as_deref().and_then(parse_reset_to_epoch_ms),
                window: Some(win.to_string()),
                status: None,
            });
        }
    }
    out
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
    fn test_anthropic_legacy_unified() {
        let h = hm(&[
            ("anthropic-ratelimit-unified-status", "allowed_warning"),
            ("anthropic-ratelimit-unified-remaining", "20"),
            ("anthropic-ratelimit-unified-limit", "100"),
            ("anthropic-ratelimit-unified-reset", "2026-01-01T00:00:00Z"),
        ]);
        let qs = anthropic_quotas_from_headers(&h);
        assert_eq!(qs.len(), 1);
        let q = &qs[0];
        assert_eq!(q.remaining, Some(20));
        assert_eq!(q.limit, Some(100));
        assert_eq!(q.used_pct, Some(80.0));
        assert_eq!(q.status.as_deref(), Some("allowed_warning"));
        assert_eq!(q.window.as_deref(), Some("unified"));
        assert_eq!(q.resets_at, Some(1_767_225_600_000));
    }

    #[test]
    fn test_anthropic_unified_multi_window() {
        let h = hm(&[
            ("anthropic-ratelimit-unified-status", "allowed_warning"),
            ("anthropic-ratelimit-unified-5h-status", "allowed"),
            ("anthropic-ratelimit-unified-5h-utilization", "0.38"),
            ("anthropic-ratelimit-unified-5h-reset", "1767225600"),
            ("anthropic-ratelimit-unified-7d-status", "allowed_warning"),
            ("anthropic-ratelimit-unified-7d-utilization", "0.81"),
            ("anthropic-ratelimit-unified-7d-reset", "1767225600"),
        ]);
        let qs = anthropic_quotas_from_headers(&h);
        assert_eq!(qs.len(), 2);
        let five = qs.iter().find(|q| q.window.as_deref() == Some("5h")).unwrap();
        assert_eq!(five.used_pct, Some(38.0));
        assert_eq!(five.status.as_deref(), Some("allowed"));
        let week = qs.iter().find(|q| q.window.as_deref() == Some("7d")).unwrap();
        assert_eq!(week.used_pct, Some(81.0));

        // Worst window is the 7d at 81% used.
        let worst = worst_quota(&qs).unwrap();
        assert_eq!(worst.window.as_deref(), Some("7d"));
    }

    #[test]
    fn test_anthropic_standard_multi() {
        let h = hm(&[
            ("anthropic-ratelimit-requests-remaining", "50"),
            ("anthropic-ratelimit-requests-limit", "100"),
            ("anthropic-ratelimit-tokens-remaining", "5000"),
            ("anthropic-ratelimit-tokens-limit", "20000"),
        ]);
        let qs = anthropic_quotas_from_headers(&h);
        assert_eq!(qs.len(), 2);
        // tokens at 75% used is worse than requests at 50%.
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("tokens"));
    }

    #[test]
    fn test_anthropic_none() {
        let h = hm(&[("content-type", "application/json")]);
        assert!(anthropic_quotas_from_headers(&h).is_empty());
    }

    #[test]
    fn test_worst_quota_prefers_exhausted() {
        let qs = vec![
            QuotaSnapshot { remaining: None, limit: None, used_pct: Some(50.0), resets_at: None, window: Some("5h".into()), status: Some("allowed".into()) },
            QuotaSnapshot { remaining: None, limit: None, used_pct: Some(20.0), resets_at: None, window: Some("7d".into()), status: Some("rejected".into()) },
        ];
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("7d"));
    }

    #[test]
    fn test_openai_multi() {
        let h = hm(&[
            ("x-ratelimit-remaining-tokens", "8000"),
            ("x-ratelimit-limit-tokens", "10000"),
            ("x-ratelimit-reset-tokens", "6m0s"),
            ("x-ratelimit-remaining-requests", "40"),
            ("x-ratelimit-limit-requests", "60"),
        ]);
        let qs = openai_quotas_from_headers(&h);
        assert_eq!(qs.len(), 2);
        // requests at 33% used is worse than tokens at 20%.
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("requests"));
    }
}
