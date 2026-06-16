//! Per-provider incremental aggregates for O(1) hot-path routing reads.
//!
//! The metrics event deque is the source of truth for the live feed, history,
//! and percentile math, but scanning it on every routing decision (O(events) per
//! provider, under a shared lock) does not scale. These aggregates are updated
//! incrementally on write so the routing hot path can read provider health,
//! availability, and rate-limit cooldown in O(1) without touching the deque.

use std::collections::VecDeque;
use std::time::Duration;

use crate::metrics::{ErrorType, HealthState, MetricsEvent};
use crate::providers::provider_trait::{CurrencyAmount, QuotaSnapshot};
use crate::providers::quota::quota_exhausted;

/// Health/backoff window (matches the legacy full-scan behavior).
const WINDOW_MS: u64 = 300_000; // 5 minutes
/// Retain failure samples slightly beyond the window before pruning.
const PRUNE_MS: u64 = 360_000; // 6 minutes
const MAX_BACKOFF_MS: u64 = 30_000;
const BASE_BACKOFF_MS: u64 = 100;
const RATE_LIMIT_DEFAULT_MS: u64 = 30_000;

#[derive(Clone, Copy)]
struct FailureSample {
    ts_ms: u64,
    rate_limit: bool,
    retry_after_ms: Option<u64>,
}

/// Lightweight rolling state for one provider.
#[derive(Default)]
pub(crate) struct ProviderAgg {
    /// Recent failures (small: failures are far rarer than load/token events).
    failures: VecDeque<FailureSample>,
    /// Most recent balance snapshot and when it was observed.
    last_balance: Option<(CurrencyAmount, u64)>,
    /// Most recent quota snapshot (all windows) and when it was observed.
    last_quota: Option<(Vec<QuotaSnapshot>, u64)>,
}

/// Routing-relevant view of a provider, computed in one shot under a single lock.
#[derive(Clone, Copy)]
pub struct RoutingHealth {
    pub health: HealthState,
    pub available: bool,
    pub retry_in: Duration,
}

impl RoutingHealth {
    /// Default for a provider we've never seen an event for.
    pub fn unknown() -> Self {
        Self {
            health: HealthState::Healthy,
            available: true,
            retry_in: Duration::ZERO,
        }
    }
}

impl ProviderAgg {
    /// Fold a recorded event into the aggregate. `ts_ms` is the event timestamp,
    /// `now_ms` is the current wall clock (used only for pruning).
    pub fn apply(&mut self, event: &MetricsEvent, ts_ms: u64, now_ms: u64) {
        match event {
            MetricsEvent::Failure(d) => self.failures.push_back(FailureSample {
                ts_ms,
                rate_limit: d.error_type == ErrorType::RateLimit,
                retry_after_ms: d.retry_after_ms,
            }),
            MetricsEvent::Balance(amount) => self.last_balance = Some((*amount, ts_ms)),
            MetricsEvent::Quota(q) => self.last_quota = Some((q.clone(), ts_ms)),
            _ => {}
        }
        self.prune(now_ms);
    }

    fn prune(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(PRUNE_MS);
        while self.failures.front().is_some_and(|f| f.ts_ms < cutoff) {
            self.failures.pop_front();
        }
    }

    fn window_start(now_ms: u64) -> u64 {
        now_ms.saturating_sub(WINDOW_MS)
    }

    pub fn failure_count(&self, now_ms: u64) -> u32 {
        let start = Self::window_start(now_ms);
        self.failures.iter().filter(|f| f.ts_ms >= start).count() as u32
    }

    fn latest_failure(&self, now_ms: u64) -> Option<FailureSample> {
        let start = Self::window_start(now_ms);
        self.failures
            .iter()
            .filter(|f| f.ts_ms >= start)
            .max_by_key(|f| f.ts_ms)
            .copied()
    }

    fn balance_issue(&self, now_ms: u64) -> bool {
        let start = Self::window_start(now_ms);
        match self.last_balance {
            Some((amount, ts)) if ts >= start => match amount {
                CurrencyAmount::Msats(m) => m <= 0,
                CurrencyAmount::Sats(s) => s <= 0,
                CurrencyAmount::UsdMicro(u) => u <= 0,
            },
            _ => false,
        }
    }

    /// Epoch-ms at which an exhausted quota window resets, if one is blocking now.
    fn quota_reset(&self, now_ms: u64) -> Option<i64> {
        let start = Self::window_start(now_ms);
        let (quotas, ts) = self.last_quota.as_ref()?;
        if *ts < start {
            return None;
        }
        quota_snapshot_issue_reset(quotas, now_ms as i64)
    }

    pub fn health(&self, now_ms: u64) -> HealthState {
        if self.balance_issue(now_ms) || self.quota_reset(now_ms).is_some() {
            HealthState::Degraded
        } else {
            let f = self.failure_count(now_ms);
            if f >= 5 {
                HealthState::Unhealthy
            } else if f >= 2 {
                HealthState::Degraded
            } else {
                HealthState::Healthy
            }
        }
    }

    /// Remaining cooldown before the provider should be retried (0 = ready now).
    pub fn time_until_retry(&self, now_ms: u64) -> Duration {
        let quota_remaining = self
            .quota_reset(now_ms)
            .map(|reset| Duration::from_millis((reset - now_ms as i64).max(0) as u64))
            .unwrap_or_default();

        let failure_remaining = self
            .latest_failure(now_ms)
            .map(|last| {
                let backoff_ms = if last.rate_limit {
                    last.retry_after_ms.unwrap_or(RATE_LIMIT_DEFAULT_MS)
                } else {
                    exponential_backoff_ms(self.failure_count(now_ms))
                };
                let ready_at = last.ts_ms.saturating_add(backoff_ms);
                Duration::from_millis(ready_at.saturating_sub(now_ms))
            })
            .unwrap_or_default();

        failure_remaining.max(quota_remaining)
    }

    /// Recommended backoff *size* (legacy semantics): exponential by failure
    /// count, floored by any quota reset window.
    pub fn backoff(&self, now_ms: u64) -> Duration {
        let quota_backoff = self
            .quota_reset(now_ms)
            .map(|reset| Duration::from_millis((reset - now_ms as i64).max(0) as u64))
            .unwrap_or_default();

        let count = self.failure_count(now_ms);
        if count == 0 {
            return quota_backoff;
        }
        Duration::from_millis(exponential_backoff_ms(count)).max(quota_backoff)
    }

    pub fn routing_health(&self, now_ms: u64) -> RoutingHealth {
        let health = self.health(now_ms);
        RoutingHealth {
            health,
            available: health != HealthState::Unhealthy,
            retry_in: self.time_until_retry(now_ms),
        }
    }

    pub fn balance(&self) -> Option<CurrencyAmount> {
        self.last_balance.map(|(a, _)| a)
    }

    pub fn quota(&self) -> Option<Vec<QuotaSnapshot>> {
        self.last_quota.as_ref().map(|(q, _)| q.clone())
    }
}

fn exponential_backoff_ms(failure_count: u32) -> u64 {
    BASE_BACKOFF_MS
        .saturating_mul(2u64.saturating_pow(failure_count.saturating_sub(1).min(10)))
        .min(MAX_BACKOFF_MS)
}

/// Latest-blocking reset across exhausted quota windows (`None` if none block).
pub(crate) fn quota_snapshot_issue_reset(quotas: &[QuotaSnapshot], now_ms: i64) -> Option<i64> {
    quotas
        .iter()
        .filter(|q| quota_exhausted(q))
        .filter_map(|q| match q.resets_at {
            Some(reset) if reset <= now_ms => None, // already recovered
            Some(reset) => Some(reset),
            None => Some(now_ms), // exhausted, no reset info => no extra backoff
        })
        .max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::FailureDetails;

    fn failure(rate_limit: bool, retry_after_ms: Option<u64>) -> MetricsEvent {
        MetricsEvent::Failure(FailureDetails {
            error_type: if rate_limit { ErrorType::RateLimit } else { ErrorType::Other },
            error_code: None,
            error_message: "x".into(),
            retry_after_ms,
            status_code: None,
        })
    }

    #[test]
    fn health_thresholds_match_legacy() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        assert_eq!(a.health(now), HealthState::Healthy);
        a.apply(&failure(false, None), now, now);
        assert_eq!(a.health(now), HealthState::Healthy); // 1 failure
        a.apply(&failure(false, None), now, now);
        assert_eq!(a.health(now), HealthState::Degraded); // 2 failures
        for _ in 0..3 {
            a.apply(&failure(false, None), now, now);
        }
        assert_eq!(a.health(now), HealthState::Unhealthy); // 5 failures
    }

    #[test]
    fn failures_outside_window_are_ignored() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        // 5 failures 6 minutes ago; pruned and out of window.
        for _ in 0..5 {
            a.apply(&failure(false, None), now - 360_001, now);
        }
        assert_eq!(a.failure_count(now), 0);
        assert_eq!(a.health(now), HealthState::Healthy);
    }

    #[test]
    fn rate_limit_uses_retry_after() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        a.apply(&failure(true, Some(60_000)), now, now);
        let r = a.time_until_retry(now).as_millis() as u64;
        assert!(r > 55_000 && r <= 60_000, "got {r}");
    }

    #[test]
    fn balance_zero_is_degraded_then_recovers() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        a.apply(&MetricsEvent::Balance(CurrencyAmount::Msats(0)), now, now);
        assert_eq!(a.health(now), HealthState::Degraded);
        // A later positive balance clears the issue (more correct than legacy scan).
        a.apply(&MetricsEvent::Balance(CurrencyAmount::Msats(500)), now, now);
        assert_eq!(a.health(now), HealthState::Healthy);
    }

    #[test]
    fn quota_exhausted_backoff_until_reset() {
        let now = 1_000_000_000u64;
        let reset = now as i64 + 3_600_000;
        let mut a = ProviderAgg::default();
        a.apply(
            &MetricsEvent::Quota(vec![QuotaSnapshot {
                remaining: None,
                limit: None,
                used_pct: Some(100.0),
                resets_at: Some(reset),
                window: Some("unified".into()),
                status: None,
            }]),
            now,
            now,
        );
        assert_eq!(a.health(now), HealthState::Degraded);
        assert!(a.backoff(now) > Duration::from_secs(60));
    }
}
