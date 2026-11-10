use std::collections::BTreeMap;

use serde::Serialize;

use crate::providers::provider_trait::QuotaSnapshot;
use crate::providers::quota::{blocking_quotas, quota_used_pct, worst_quota};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuotaWindow {
    pub provider: Option<String>,
    pub window: Option<String>,
    pub used_pct: Option<f32>,
    pub resets_at: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuotaSummary {
    pub used_pct: Option<f32>,
    pub window: Option<String>,
    pub resets_at: Option<i64>,
    pub exhausted: bool,
    pub next_available_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelQuota {
    pub summary: Option<QuotaSummary>,
    pub windows: Vec<QuotaWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuotaReport {
    pub windows: Vec<QuotaWindow>,
    pub summary: Option<QuotaSummary>,
    pub models: BTreeMap<String, ModelQuota>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServingQuota {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub window: Option<String>,
    pub used_pct: Option<f32>,
    pub resets_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaPick {
    Lowest,
    FirstOpen,
}

#[derive(Clone)]
pub struct ProviderQuota {
    pub label: String,
    pub quotas: Vec<QuotaSnapshot>,
}

pub fn serving_quota(quotas: &[QuotaSnapshot]) -> Option<ServingQuota> {
    let worst = worst_quota(quotas)?;
    Some(ServingQuota {
        model: None,
        used_pct: quota_used_pct(&worst),
        window: worst.window,
        resets_at: worst.resets_at,
    })
}

fn live_blockers(quotas: &[QuotaSnapshot], now_ms: i64) -> Vec<&QuotaSnapshot> {
    blocking_quotas(quotas)
        .into_iter()
        .filter(|q| q.resets_at.is_none_or(|r| r > now_ms))
        .collect()
}

fn frees_at(blockers: &[&QuotaSnapshot], now_ms: i64) -> i64 {
    blockers.iter().map(|q| q.resets_at.unwrap_or(now_ms)).max().unwrap_or(now_ms)
}

fn window_of(provider: Option<String>, q: &QuotaSnapshot) -> QuotaWindow {
    QuotaWindow {
        provider,
        window: q.window.clone(),
        used_pct: quota_used_pct(q),
        resets_at: q.resets_at,
        status: q.status.clone(),
    }
}

fn summarize(providers: &[ProviderQuota], pick: QuotaPick, now_ms: i64) -> Option<QuotaSummary> {
    let tracked: Vec<&ProviderQuota> = providers.iter().filter(|p| !p.quotas.is_empty()).collect();
    if tracked.is_empty() {
        return None;
    }
    let untracked = providers.len() > tracked.len();

    let mut open: Vec<(&ProviderQuota, ServingQuota)> = Vec::new();
    let mut blocked: Vec<(i64, QuotaSnapshot)> = Vec::new();
    for p in &tracked {
        let blockers = live_blockers(&p.quotas, now_ms);
        if blockers.is_empty() {
            if let Some(tight) = serving_quota(&p.quotas) {
                open.push((p, tight));
            }
        } else {
            let at = frees_at(&blockers, now_ms);
            let tightest = blockers
                .iter()
                .find(|q| q.resets_at == Some(at))
                .or(blockers.first())
                .map(|q| (*q).clone())
                .unwrap_or_default();
            blocked.push((at, tightest));
        }
    }

    let chosen = match pick {
        QuotaPick::FirstOpen => open.into_iter().next(),
        QuotaPick::Lowest => open.into_iter().min_by(|a, b| {
            a.1.used_pct
                .unwrap_or(0.0)
                .partial_cmp(&b.1.used_pct.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    };
    if let Some((_, tight)) = chosen {
        return Some(QuotaSummary {
            used_pct: tight.used_pct,
            window: tight.window,
            resets_at: tight.resets_at,
            exhausted: false,
            next_available_at: None,
        });
    }

    let (at, q) = blocked.into_iter().min_by_key(|(at, _)| *at)?;
    Some(QuotaSummary {
        used_pct: quota_used_pct(&q).or(Some(100.0)),
        window: q.window,
        resets_at: q.resets_at,
        exhausted: !untracked,
        next_available_at: (!untracked).then_some(at),
    })
}

pub fn summary_of(providers: &[ProviderQuota], now_ms: i64) -> Option<QuotaSummary> {
    summarize(providers, QuotaPick::Lowest, now_ms)
}

pub fn build_model(providers: &[ProviderQuota], pick: QuotaPick, reveal_providers: bool, now_ms: i64) -> ModelQuota {
    let report = build_report(providers.to_vec(), reveal_providers, now_ms);
    let summary = summarize(providers, pick, now_ms);
    ModelQuota { summary, windows: report.windows }
}

pub fn build_report(providers: Vec<ProviderQuota>, reveal_providers: bool, now_ms: i64) -> QuotaReport {
    let summary = summarize(&providers, QuotaPick::Lowest, now_ms);
    let windows = providers
        .iter()
        .flat_map(|p| {
            let label = reveal_providers.then(|| p.label.clone());
            p.quotas.iter().map(move |q| window_of(label.clone(), q))
        })
        .collect();
    QuotaReport { windows, summary, models: BTreeMap::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000_000_000;

    fn snap(window: &str, used: f32, status: &str, resets_in: i64) -> QuotaSnapshot {
        QuotaSnapshot {
            window: Some(window.into()),
            used_pct: Some(used),
            status: Some(status.into()),
            resets_at: Some(NOW + resets_in),
            ..Default::default()
        }
    }

    fn five_x() -> ProviderQuota {
        ProviderQuota {
            label: "claude-max-5x".into(),
            quotas: vec![snap("5h", 2.0, "allowed", 3_600_000), snap("7d", 17.0, "allowed", 200_000_000)],
        }
    }

    fn ten_x_blocked() -> ProviderQuota {
        ProviderQuota {
            label: "claude-max-10x".into(),
            quotas: vec![snap("5h", 0.0, "allowed", 3_600_000), snap("7d", 100.0, "rejected", 387_042_000)],
        }
    }

    #[test]
    fn summary_uses_the_best_routable_provider() {
        let report = build_report(vec![ten_x_blocked(), five_x()], true, NOW);
        let summary = report.summary.unwrap();
        assert!(!summary.exhausted);
        assert_eq!(summary.next_available_at, None);
        assert_eq!(summary.window.as_deref(), Some("7d"));
        assert_eq!(summary.used_pct, Some(17.0));
        assert_eq!(summary.resets_at, Some(NOW + 200_000_000));
        assert_eq!(report.windows.len(), 4);
    }

    fn tiered(first_rejected: bool) -> Vec<ProviderQuota> {
        let first = if first_rejected {
            snap("5h", 100.0, "rejected", 1_800_000)
        } else {
            snap("5h", 95.0, "allowed", 1_800_000)
        };
        vec![
            ProviderQuota { label: "p1".into(), quotas: vec![first] },
            ProviderQuota { label: "p2".into(), quotas: vec![snap("5h", 10.0, "allowed", 3_600_000)] },
        ]
    }

    #[test]
    fn priority_model_reports_the_first_open_provider() {
        let model = build_model(&tiered(false), QuotaPick::FirstOpen, true, NOW);
        assert_eq!(model.summary.unwrap().used_pct, Some(95.0));
        let model = build_model(&tiered(true), QuotaPick::FirstOpen, true, NOW);
        let summary = model.summary.unwrap();
        assert_eq!(summary.used_pct, Some(10.0));
        assert_eq!(summary.resets_at, Some(NOW + 3_600_000));
        assert!(!summary.exhausted);
    }

    #[test]
    fn round_robin_model_reports_the_lowest_provider() {
        let model = build_model(&tiered(false), QuotaPick::Lowest, true, NOW);
        assert_eq!(model.summary.unwrap().used_pct, Some(10.0));
    }

    #[test]
    fn all_blocked_reports_the_earliest_reset() {
        let mut soon = five_x();
        soon.quotas = vec![snap("5h", 100.0, "rejected", 1_800_000), snap("7d", 30.0, "allowed", 200_000_000)];
        let report = build_report(vec![ten_x_blocked(), soon], true, NOW);
        let summary = report.summary.unwrap();
        assert!(summary.exhausted);
        assert_eq!(summary.next_available_at, Some(NOW + 1_800_000));
        assert_eq!(summary.window.as_deref(), Some("5h"));
        assert_eq!(summary.used_pct, Some(100.0));
    }

    #[test]
    fn a_reset_in_the_past_no_longer_blocks() {
        let mut stale = ten_x_blocked();
        stale.quotas = vec![snap("7d", 100.0, "rejected", -1)];
        let summary = build_report(vec![stale], true, NOW).summary.unwrap();
        assert!(!summary.exhausted);
    }

    #[test]
    fn plain_keys_get_no_provider_names() {
        let report = build_report(vec![five_x()], false, NOW);
        assert!(report.windows.iter().all(|w| w.provider.is_none()));
        assert_eq!(report.windows[0].used_pct, Some(2.0));
        let admin = build_report(vec![five_x()], true, NOW);
        assert_eq!(admin.windows[0].provider.as_deref(), Some("claude-max-5x"));
    }

    #[test]
    fn no_quota_data_means_no_summary() {
        let none = ProviderQuota { label: "p".into(), quotas: vec![] };
        let report = build_report(vec![none], true, NOW);
        assert!(report.windows.is_empty());
        assert!(report.summary.is_none());
    }

    #[test]
    fn an_untracked_provider_keeps_the_pool_open() {
        let none = ProviderQuota { label: "p".into(), quotas: vec![] };
        let report = build_report(vec![ten_x_blocked(), none], true, NOW);
        assert!(!report.summary.unwrap().exhausted);
    }
}
