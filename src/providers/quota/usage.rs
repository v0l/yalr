use serde::Deserialize;

use super::{parse_reset_to_epoch_ms, EXTRA_USAGE_WINDOW};
use crate::providers::provider_trait::{QuotaCredit, QuotaSnapshot};

/// A single rate-limit window in the Anthropic OAuth usage endpoint response.
/// Both `claude.ai/api/organizations/{org}/usage` and
/// `api.anthropic.com/api/oauth/usage` return the same shape.
#[derive(Debug, Deserialize)]
struct UsageWindow {
    /// Percentage of the window consumed, as a float in `[0, 100]`.
    #[serde(default)]
    utilization: Option<f64>,
    /// RFC 3339 UTC timestamp when the window rolls over (may be null).
    #[serde(default)]
    resets_at: Option<String>,
}

/// Response body of the Anthropic OAuth usage endpoint. Each field is an
/// independent rate-limit bucket; any may be absent or `null`.
#[derive(Debug, Deserialize)]
struct AnthropicUsageResponse {
    #[serde(default)]
    five_hour: Option<UsageWindow>,
    #[serde(default)]
    seven_day: Option<UsageWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<UsageWindow>,
    #[serde(default)]
    seven_day_opus: Option<UsageWindow>,
    #[serde(default)]
    extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: bool,
    #[serde(default)]
    monthly_limit: Option<f64>,
    #[serde(default)]
    used_credits: Option<f64>,
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(default)]
    decimal_places: Option<u32>,
    #[serde(default)]
    spend_limit_reached: bool,
}

fn extra_usage_quota(extra: ExtraUsage) -> Option<QuotaSnapshot> {
    if !extra.is_enabled {
        return None;
    }
    let used = extra.used_credits.unwrap_or(0.0).round() as i64;
    let limit = extra.monthly_limit.map(|l| l.round() as i64);
    let used_pct = extra
        .utilization
        .map(|u| u as f32)
        .or_else(|| limit.filter(|l| *l > 0).map(|l| used as f32 / l as f32 * 100.0));
    let exhausted = extra.spend_limit_reached || limit.is_some_and(|l| used >= l);
    Some(QuotaSnapshot {
        remaining: limit.map(|l| (l - used).max(0)),
        limit,
        used_pct,
        resets_at: None,
        window: Some(EXTRA_USAGE_WINDOW.to_string()),
        status: Some(if exhausted { "rejected" } else { "allowed" }.to_string()),
        credit: extra.currency.map(|currency| QuotaCredit {
            currency,
            decimals: extra.decimal_places.unwrap_or(2),
            used_minor: used,
            limit_minor: limit,
        }),
    })
}

/// Derive a status hint from a utilization percentage, since the usage endpoint
/// does not report one directly. Mirrors the `status` values Anthropic sends on
/// rate-limit headers so downstream display/exhaustion logic behaves uniformly.
fn status_from_pct(pct: f32) -> String {
    if pct >= 100.0 {
        "rejected".to_string()
    } else if pct >= 80.0 {
        "allowed_warning".to_string()
    } else {
        "allowed".to_string()
    }
}

/// Parse the Anthropic OAuth usage endpoint JSON body into quota windows.
///
/// This is the dedicated usage endpoint (`GET .../api/oauth/usage` for OAuth
/// tokens, or `GET .../api/organizations/{org}/usage` for web sessions) which
/// reports per-window `utilization` (0–100) plus a reset timestamp — richer and
/// more reliable than scraping rate-limit response headers. Windows that are
/// absent or `null` are skipped. Returns empty on parse failure.
pub fn anthropic_quotas_from_usage(body: &str) -> Vec<QuotaSnapshot> {
    let resp: AnthropicUsageResponse = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (win, window) in [
        ("5h", resp.five_hour),
        ("7d", resp.seven_day),
        ("7d_sonnet", resp.seven_day_sonnet),
        ("7d_opus", resp.seven_day_opus),
    ] {
        let Some(window) = window else { continue };
        let used_pct = window.utilization.map(|u| u as f32);
        out.push(QuotaSnapshot {
            remaining: None,
            limit: None,
            used_pct,
            resets_at: window.resets_at.as_deref().and_then(parse_reset_to_epoch_ms),
            window: Some(win.to_string()),
            status: used_pct.map(status_from_pct),
            credit: None,
        });
    }
    out.extend(resp.extra_usage.and_then(extra_usage_quota));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::quota::{blocking_quotas, quota_exhausted, worst_quota};

    #[test]
    fn test_anthropic_usage_endpoint() {
        let body = r#"{
            "five_hour": { "utilization": 17.0, "resets_at": "2026-01-01T00:00:00Z" },
            "seven_day": { "utilization": 85.0, "resets_at": "2026-01-01T00:00:00Z" },
            "seven_day_sonnet": { "utilization": 0.0, "resets_at": null },
            "seven_day_opus": { "utilization": 100.0, "resets_at": "2026-01-01T00:00:00Z" },
            "seven_day_oauth_apps": null,
            "extra_usage": null
        }"#;
        let qs = anthropic_quotas_from_usage(body);
        assert_eq!(qs.len(), 4);

        let five = qs.iter().find(|q| q.window.as_deref() == Some("5h")).unwrap();
        assert_eq!(five.used_pct, Some(17.0));
        assert_eq!(five.status.as_deref(), Some("allowed"));
        assert_eq!(five.resets_at, Some(1_767_225_600_000));

        let week = qs.iter().find(|q| q.window.as_deref() == Some("7d")).unwrap();
        assert_eq!(week.used_pct, Some(85.0));
        assert_eq!(week.status.as_deref(), Some("allowed_warning"));

        let sonnet = qs.iter().find(|q| q.window.as_deref() == Some("7d_sonnet")).unwrap();
        assert_eq!(sonnet.resets_at, None);
        assert_eq!(sonnet.status.as_deref(), Some("allowed"));

        let opus = qs.iter().find(|q| q.window.as_deref() == Some("7d_opus")).unwrap();
        assert_eq!(opus.status.as_deref(), Some("rejected"));
        assert!(quota_exhausted(opus));

        // Worst window is the exhausted opus bucket.
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("7d_opus"));
    }

    #[test]
    fn test_anthropic_usage_partial_and_invalid() {
        // Only some buckets present.
        let qs = anthropic_quotas_from_usage(
            r#"{ "five_hour": { "utilization": 6.0, "resets_at": "2026-01-01T00:00:00Z" } }"#,
        );
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].window.as_deref(), Some("5h"));

        // Invalid JSON yields an empty vec rather than panicking.
        assert!(anthropic_quotas_from_usage("not json").is_empty());
        assert!(anthropic_quotas_from_usage("{}").is_empty());
    }

    #[test]
    fn test_extra_usage_unlimited_covers_exhausted_plan() {
        let body = r#"{
            "five_hour": { "utilization": 0.0, "resets_at": null },
            "seven_day": { "utilization": 100.0, "resets_at": "2099-01-01T00:00:00Z" },
            "extra_usage": { "is_enabled": true, "monthly_limit": null, "used_credits": 5348.0,
                "utilization": null, "currency": "EUR", "decimal_places": 2, "spend_limit_reached": false }
        }"#;
        let qs = anthropic_quotas_from_usage(body);
        let extra = qs.iter().find(|q| q.window.as_deref() == Some("extra")).unwrap();
        assert_eq!(extra.status.as_deref(), Some("allowed"));
        let credit = extra.credit.as_ref().unwrap();
        assert_eq!((credit.currency.as_str(), credit.used_minor, credit.limit_minor), ("EUR", 5348, None));
        assert!(blocking_quotas(&qs).is_empty());
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("extra"));
    }

    #[test]
    fn test_extra_usage_limit_reached_blocks() {
        let body = r#"{
            "seven_day": { "utilization": 100.0, "resets_at": "2099-01-01T00:00:00Z" },
            "extra_usage": { "is_enabled": true, "monthly_limit": 1000.0, "used_credits": 1000.0,
                "currency": "USD", "decimal_places": 2, "spend_limit_reached": true }
        }"#;
        let qs = anthropic_quotas_from_usage(body);
        let extra = qs.iter().find(|q| q.window.as_deref() == Some("extra")).unwrap();
        assert_eq!((extra.remaining, extra.limit, extra.used_pct), (Some(0), Some(1000), Some(100.0)));
        assert_eq!(blocking_quotas(&qs).len(), 1);
        assert_eq!(worst_quota(&qs).unwrap().window.as_deref(), Some("7d"));
    }

    #[test]
    fn test_extra_usage_disabled_is_skipped() {
        let body = r#"{
            "five_hour": { "utilization": 100.0, "resets_at": "2099-01-01T00:00:00Z" },
            "extra_usage": { "is_enabled": false, "monthly_limit": null, "used_credits": null }
        }"#;
        let qs = anthropic_quotas_from_usage(body);
        assert_eq!(qs.len(), 1);
        assert_eq!(blocking_quotas(&qs).len(), 1);
    }
}
