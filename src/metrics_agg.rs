//! Per-provider incremental aggregates for O(1) hot-path routing reads.
//!
//! The metrics event deque is the source of truth for the live feed, history,
//! and percentile math, but scanning it on every routing decision (O(events) per
//! provider, under a shared lock) does not scale. These aggregates are updated
//! incrementally on write so the routing hot path can read provider health,
//! availability, and rate-limit cooldown without touching the deque.
//!
//! Health is rate based: only provider-attributed faults count, one caller can
//! contribute a bounded number of them, and a provider that passes health
//! probes after a fault run is forgiven the earlier faults.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::metrics::{ErrorType, HealthState, MetricsEvent, MetricsUser};
use crate::providers::attribution::FailureAttribution;
use crate::providers::provider_trait::{CurrencyAmount, QuotaSnapshot};
use crate::providers::quota::blocking_quotas;

const WINDOW_MS: u64 = 300_000;
const PRUNE_MS: u64 = 360_000;
const MAX_BACKOFF_MS: u64 = 30_000;
const BASE_BACKOFF_MS: u64 = 100;
const RATE_LIMIT_DEFAULT_MS: u64 = 30_000;
const MAX_PROBE_STREAK: usize = 16;
const RECENT_FAULTS_SHOWN: usize = 5;
const MESSAGE_CHARS: usize = 200;
const SYSTEM_CALLER: i64 = i64::MIN;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthThresholds {
    pub degraded_ratio: f64,
    pub degraded_min_attempts: u32,
    pub degraded_consecutive: u32,
    pub degraded_consecutive_users: u32,
    pub unhealthy_ratio: f64,
    pub unhealthy_min_attempts: u32,
    pub unhealthy_connection_streak: u32,
    pub per_user_cap: u32,
    pub recovery_probe_passes: u32,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            degraded_ratio: 0.2,
            degraded_min_attempts: 5,
            degraded_consecutive: 3,
            degraded_consecutive_users: 2,
            unhealthy_ratio: 0.5,
            unhealthy_min_attempts: 10,
            unhealthy_connection_streak: 3,
            per_user_cap: 2,
            recovery_probe_passes: 2,
        }
    }
}

#[derive(Clone)]
struct FaultSample {
    attribution: FailureAttribution,
    connection: bool,
    rate_limit: bool,
    retry_after_ms: Option<u64>,
    message: String,
}

#[derive(Clone)]
enum Outcome {
    Success,
    Fault(FaultSample),
}

#[derive(Clone)]
struct Sample {
    ts_ms: u64,
    caller: Option<i64>,
    outcome: Outcome,
}

#[derive(Default)]
pub(crate) struct ProviderAgg {
    samples: VecDeque<Sample>,
    probe_passes: Vec<u64>,
    last_balance: Option<(CurrencyAmount, u64)>,
    last_quota: Option<(Vec<QuotaSnapshot>, u64)>,
    slow_start: Option<u64>,
}

#[derive(Clone, Copy)]
pub struct RoutingHealth {
    pub health: HealthState,
    pub available: bool,
    pub retry_in: Duration,
}

impl RoutingHealth {
    pub fn unknown() -> Self {
        Self {
            health: HealthState::Healthy,
            available: true,
            retry_in: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecentFault {
    pub ago_ms: u64,
    pub attribution: FailureAttribution,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReason {
    pub cause: &'static str,
    pub fault_ratio: f64,
    pub attempts: u32,
    pub provider_faults: u32,
    pub unknown_faults: u32,
    pub request_faults: u32,
    pub capped_faults: u32,
    pub consecutive_provider_faults: u32,
    pub probe_passes: u32,
    pub probe_passes_needed: u32,
    pub recent_faults: Vec<RecentFault>,
}

#[derive(Default)]
struct Tally {
    successes: u32,
    provider_faults: u32,
    unknown_faults: u32,
    request_faults: u32,
    capped_faults: u32,
    consecutive_provider: u32,
    consecutive_users: u32,
    connection_streak: u32,
}

impl Tally {
    fn attempts(&self) -> u32 {
        self.successes + self.provider_faults + self.unknown_faults
    }

    fn fault_ratio(&self) -> f64 {
        let attempts = self.attempts();
        if attempts == 0 {
            return 0.0;
        }
        let weighted = self.provider_faults as f64
            + self.unknown_faults as f64 * FailureAttribution::Unknown.weight();
        weighted / attempts as f64
    }

    fn state(&self, th: &HealthThresholds) -> (HealthState, &'static str) {
        let attempts = self.attempts();
        let ratio = self.fault_ratio();
        if self.connection_streak >= th.unhealthy_connection_streak {
            (HealthState::Unhealthy, "connection_failures")
        } else if attempts >= th.unhealthy_min_attempts && ratio >= th.unhealthy_ratio {
            (HealthState::Unhealthy, "fault_ratio")
        } else if attempts >= th.degraded_min_attempts && ratio >= th.degraded_ratio {
            (HealthState::Degraded, "fault_ratio")
        } else if self.consecutive_provider >= th.degraded_consecutive
            && self.consecutive_users >= th.degraded_consecutive_users
        {
            (HealthState::Degraded, "consecutive_faults")
        } else {
            (HealthState::Healthy, "none")
        }
    }
}

fn caller_key(user: Option<&MetricsUser>) -> Option<i64> {
    user.and_then(|u| u.id.or(u.api_key_id.map(|k| -k - 1)))
}

fn clip(message: &str) -> String {
    message.chars().take(MESSAGE_CHARS).collect()
}

impl ProviderAgg {
    pub fn apply(&mut self, event: &MetricsEvent, user: Option<&MetricsUser>, ts_ms: u64, now_ms: u64) {
        match event {
            MetricsEvent::Failure(d) => self.samples.push_back(Sample {
                ts_ms,
                caller: caller_key(user),
                outcome: Outcome::Fault(FaultSample {
                    attribution: d.attribution,
                    connection: d.connection,
                    rate_limit: d.error_type == ErrorType::RateLimit,
                    retry_after_ms: d.retry_after_ms,
                    message: clip(&d.error_message),
                }),
            }),
            MetricsEvent::Success => self.samples.push_back(Sample {
                ts_ms,
                caller: caller_key(user),
                outcome: Outcome::Success,
            }),
            MetricsEvent::Probe { passed: true } => {
                if self.probe_passes.len() < MAX_PROBE_STREAK {
                    self.probe_passes.push(ts_ms);
                }
            }
            MetricsEvent::Probe { passed: false } => self.probe_passes.clear(),
            MetricsEvent::Balance(amount) => self.last_balance = Some((*amount, ts_ms)),
            MetricsEvent::Quota(q) => self.last_quota = Some((q.clone(), ts_ms)),
            MetricsEvent::TtftDeadline { exceeded: true } => self.slow_start = Some(ts_ms),
            MetricsEvent::TtftDeadline { exceeded: false } => self.slow_start = None,
            _ => {}
        }
        self.prune(now_ms);
    }

    fn prune(&mut self, now_ms: u64) {
        let cutoff = now_ms.saturating_sub(PRUNE_MS);
        while self.samples.front().is_some_and(|s| s.ts_ms < cutoff) {
            self.samples.pop_front();
        }
    }

    fn window_start(now_ms: u64) -> u64 {
        now_ms.saturating_sub(WINDOW_MS)
    }

    fn forgiven_before(&self, th: &HealthThresholds) -> u64 {
        let needed = th.recovery_probe_passes as usize;
        if needed > 0 && self.probe_passes.len() >= needed {
            self.probe_passes[needed - 1]
        } else {
            0
        }
    }

    fn live_samples<'a>(&'a self, now_ms: u64, forgiven_before: u64) -> impl Iterator<Item = &'a Sample> {
        let start = Self::window_start(now_ms);
        self.samples
            .iter()
            .filter(move |s| s.ts_ms >= start && s.ts_ms > forgiven_before)
    }

    fn tally(&self, now_ms: u64, th: &HealthThresholds) -> Tally {
        let mut tally = Tally::default();
        let mut per_caller: HashMap<i64, u32> = HashMap::new();
        let mut counted: Vec<(&FaultSample, Option<i64>)> = Vec::new();
        let mut order: Vec<Option<(&FaultSample, i64)>> = Vec::new();
        for sample in self.live_samples(now_ms, self.forgiven_before(th)) {
            match &sample.outcome {
                Outcome::Success => {
                    tally.successes += 1;
                    order.push(None);
                }
                Outcome::Fault(f) if f.attribution == FailureAttribution::Request => tally.request_faults += 1,
                Outcome::Fault(f) => {
                    if let (false, Some(key)) = (f.connection, sample.caller) {
                        let seen = per_caller.entry(key).or_default();
                        if *seen >= th.per_user_cap {
                            tally.capped_faults += 1;
                            continue;
                        }
                        *seen += 1;
                    }
                    match f.attribution {
                        FailureAttribution::Provider => tally.provider_faults += 1,
                        _ => tally.unknown_faults += 1,
                    }
                    counted.push((f, sample.caller));
                    order.push(Some((f, sample.caller.unwrap_or(SYSTEM_CALLER))));
                }
            }
        }
        let mut users = HashSet::new();
        for entry in order.iter().rev() {
            match entry {
                Some((f, user)) if f.attribution == FailureAttribution::Provider => {
                    tally.consecutive_provider += 1;
                    users.insert(*user);
                }
                _ => break,
            }
        }
        tally.consecutive_users = users.len() as u32;
        for entry in order.iter().rev() {
            match entry {
                Some((f, _)) if f.connection => tally.connection_streak += 1,
                _ => break,
            }
        }
        tally
    }

    pub fn failure_count(&self, now_ms: u64) -> u32 {
        self.live_samples(now_ms, 0)
            .filter(|s| matches!(&s.outcome, Outcome::Fault(f) if f.attribution != FailureAttribution::Request))
            .count() as u32
    }

    fn latest_failure(&self, now_ms: u64) -> Option<(u64, FaultSample)> {
        self.live_samples(now_ms, 0)
            .filter_map(|s| match &s.outcome {
                Outcome::Fault(f) if f.attribution != FailureAttribution::Request => Some((s.ts_ms, f.clone())),
                _ => None,
            })
            .max_by_key(|(ts, _)| *ts)
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

    fn quota_reset(&self, now_ms: u64) -> Option<i64> {
        let start = Self::window_start(now_ms);
        let (quotas, ts) = self.last_quota.as_ref()?;
        let reset = quota_snapshot_issue_reset(quotas, now_ms as i64)?;
        if *ts < start && reset <= now_ms as i64 {
            return None;
        }
        Some(reset)
    }

    fn slow_start_live(&self, now_ms: u64, th: &HealthThresholds) -> bool {
        let forgiven = self.forgiven_before(th);
        self.slow_start
            .is_some_and(|ts| ts >= Self::window_start(now_ms) && ts > forgiven)
    }

    pub fn health(&self, now_ms: u64, th: &HealthThresholds) -> HealthState {
        if self.balance_issue(now_ms) || self.quota_reset(now_ms).is_some() {
            return HealthState::Degraded;
        }
        match self.tally(now_ms, th).state(th).0 {
            HealthState::Healthy if self.slow_start_live(now_ms, th) => HealthState::Degraded,
            state => state,
        }
    }

    pub fn explain(&self, now_ms: u64, th: &HealthThresholds) -> HealthReason {
        let tally = self.tally(now_ms, th);
        let (state, mut cause) = tally.state(th);
        if state == HealthState::Healthy && self.slow_start_live(now_ms, th) {
            cause = "slow_first_token";
        }
        if self.balance_issue(now_ms) {
            cause = "balance";
        } else if self.quota_reset(now_ms).is_some() {
            cause = "quota";
        }
        let forgiven = self.forgiven_before(th);
        let mut recent_faults: Vec<RecentFault> = self
            .live_samples(now_ms, forgiven)
            .filter_map(|s| match &s.outcome {
                Outcome::Fault(f) => Some(RecentFault {
                    ago_ms: now_ms.saturating_sub(s.ts_ms),
                    attribution: f.attribution,
                    message: f.message.clone(),
                }),
                Outcome::Success => None,
            })
            .collect();
        recent_faults.reverse();
        recent_faults.truncate(RECENT_FAULTS_SHOWN);
        HealthReason {
            cause,
            fault_ratio: tally.fault_ratio(),
            attempts: tally.attempts(),
            provider_faults: tally.provider_faults,
            unknown_faults: tally.unknown_faults,
            request_faults: tally.request_faults,
            capped_faults: tally.capped_faults,
            consecutive_provider_faults: tally.consecutive_provider,
            probe_passes: self.probe_passes.len() as u32,
            probe_passes_needed: th.recovery_probe_passes,
            recent_faults,
        }
    }

    pub fn time_until_retry(&self, now_ms: u64) -> Duration {
        let quota_remaining = self
            .quota_reset(now_ms)
            .map(|reset| Duration::from_millis((reset - now_ms as i64).max(0) as u64))
            .unwrap_or_default();

        let failure_remaining = self
            .latest_failure(now_ms)
            .map(|(ts_ms, last)| {
                let backoff_ms = if last.rate_limit {
                    last.retry_after_ms.unwrap_or(RATE_LIMIT_DEFAULT_MS)
                } else {
                    exponential_backoff_ms(self.failure_count(now_ms))
                };
                let ready_at = ts_ms.saturating_add(backoff_ms);
                Duration::from_millis(ready_at.saturating_sub(now_ms))
            })
            .unwrap_or_default();

        failure_remaining.max(quota_remaining)
    }

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

    pub fn routing_health(&self, now_ms: u64, th: &HealthThresholds) -> RoutingHealth {
        let health = self.health(now_ms, th);
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

pub struct HealthReplay {
    agg: ProviderAgg,
    thresholds: HealthThresholds,
}

impl HealthReplay {
    pub fn new(thresholds: HealthThresholds) -> Self {
        Self { agg: ProviderAgg::default(), thresholds }
    }

    pub fn record(&mut self, event: &MetricsEvent, user: Option<&MetricsUser>, ts_ms: u64) {
        self.agg.apply(event, user, ts_ms, ts_ms);
    }

    pub fn health_at(&self, ts_ms: u64) -> HealthState {
        self.agg.health(ts_ms, &self.thresholds)
    }
}

fn exponential_backoff_ms(failure_count: u32) -> u64 {
    BASE_BACKOFF_MS
        .saturating_mul(2u64.saturating_pow(failure_count.saturating_sub(1).min(10)))
        .min(MAX_BACKOFF_MS)
}

pub(crate) fn quota_snapshot_issue_reset(quotas: &[QuotaSnapshot], now_ms: i64) -> Option<i64> {
    blocking_quotas(quotas)
        .into_iter()
        .filter_map(|q| match q.resets_at {
            Some(reset) if reset <= now_ms => None,
            Some(reset) => Some(reset),
            None => Some(now_ms),
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
            attribution: FailureAttribution::Provider,
            connection: false,
        })
    }

    fn user(id: i64) -> MetricsUser {
        MetricsUser { id: Some(id), name: None, api_key_id: None, api_key_name: None }
    }

    fn fail_as(a: &mut ProviderAgg, caller: i64, now: u64) {
        a.apply(&failure(false, None), Some(&user(caller)), now, now);
    }

    fn pass(a: &mut ProviderAgg, caller: i64, now: u64) {
        a.apply(&MetricsEvent::Success, Some(&user(caller)), now, now);
    }

    #[test]
    fn health_is_rate_based_with_a_minimum_sample() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        assert_eq!(a.health(now, &th), HealthState::Healthy);
        for caller in 0..2 {
            fail_as(&mut a, caller, now);
        }
        assert_eq!(a.health(now, &th), HealthState::Healthy);
        for caller in 2..10 {
            fail_as(&mut a, caller, now);
        }
        assert_eq!(a.health(now, &th), HealthState::Unhealthy);
    }

    #[test]
    fn failures_outside_window_are_ignored() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        for caller in 0..5 {
            a.apply(&failure(false, None), Some(&user(caller)), now - 360_001, now);
        }
        assert_eq!(a.failure_count(now), 0);
        assert_eq!(a.health(now, &HealthThresholds::default()), HealthState::Healthy);
    }

    #[test]
    fn one_caller_is_counted_at_most_twice_per_window() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        for _ in 0..20 {
            fail_as(&mut a, 1, now);
        }
        let reason = a.explain(now, &th);
        assert_eq!((reason.provider_faults, reason.capped_faults), (2, 18));
        assert_eq!(a.health(now, &th), HealthState::Healthy);
    }

    #[test]
    fn two_probe_passes_forgive_earlier_faults() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        for caller in 0..3 {
            fail_as(&mut a, caller, now);
        }
        assert_eq!(a.health(now + 1, &th), HealthState::Degraded);
        a.apply(&MetricsEvent::Probe { passed: true }, None, now + 1, now + 1);
        assert_eq!(a.health(now + 1, &th), HealthState::Degraded);
        a.apply(&MetricsEvent::Probe { passed: true }, None, now + 2, now + 2);
        assert_eq!(a.health(now + 2, &th), HealthState::Healthy);
    }

    #[test]
    fn a_failed_probe_restarts_the_pass_count() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        for caller in 0..3 {
            fail_as(&mut a, caller, now);
        }
        a.apply(&MetricsEvent::Probe { passed: true }, None, now + 1, now + 1);
        a.apply(&MetricsEvent::Probe { passed: false }, None, now + 2, now + 2);
        a.apply(&MetricsEvent::Probe { passed: true }, None, now + 3, now + 3);
        assert_eq!(a.health(now + 3, &th), HealthState::Degraded);
    }

    #[test]
    fn successes_dilute_the_fault_ratio() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        for caller in 0..100 {
            pass(&mut a, caller, now);
        }
        for caller in 0..3 {
            fail_as(&mut a, 200 + caller, now);
        }
        for caller in 0..2 {
            pass(&mut a, caller, now);
        }
        assert_eq!(a.health(now, &th), HealthState::Healthy);
    }

    #[test]
    fn rate_limit_uses_retry_after() {
        let now = 1_000_000_000;
        let mut a = ProviderAgg::default();
        a.apply(&failure(true, Some(60_000)), None, now, now);
        let r = a.time_until_retry(now).as_millis() as u64;
        assert!(r > 55_000 && r <= 60_000, "got {r}");
    }

    #[test]
    fn balance_zero_is_degraded_then_recovers() {
        let now = 1_000_000_000;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        a.apply(&MetricsEvent::Balance(CurrencyAmount::Msats(0)), None, now, now);
        assert_eq!(a.health(now, &th), HealthState::Degraded);
        a.apply(&MetricsEvent::Balance(CurrencyAmount::Msats(500)), None, now, now);
        assert_eq!(a.health(now, &th), HealthState::Healthy);
    }

    #[test]
    fn quota_exhausted_backoff_until_reset() {
        let now = 1_000_000_000u64;
        let th = HealthThresholds::default();
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
                credit: None,
            }]),
            None,
            now,
            now,
        );
        assert_eq!(a.health(now, &th), HealthState::Degraded);
        assert!(a.backoff(now) > Duration::from_secs(60));
    }

    #[test]
    fn a_stale_snapshot_keeps_blocking_until_its_reset() {
        let now = 1_000_000_000u64;
        let th = HealthThresholds::default();
        let reset = now as i64 + 387_042_000;
        let mut a = ProviderAgg::default();
        let seen_at = now - 3_600_000;
        a.apply(
            &MetricsEvent::Quota(vec![QuotaSnapshot {
                used_pct: Some(100.0),
                resets_at: Some(reset),
                window: Some("7d".into()),
                status: Some("rejected".into()),
                ..Default::default()
            }]),
            None,
            seen_at,
            seen_at,
        );
        assert_eq!(a.health(now, &th), HealthState::Degraded);
        assert!(a.time_until_retry(now) > Duration::from_secs(300_000));
    }

    #[test]
    fn extra_usage_keeps_exhausted_plan_healthy() {
        let now = 1_000_000_000u64;
        let th = HealthThresholds::default();
        let mut a = ProviderAgg::default();
        a.apply(
            &MetricsEvent::Quota(vec![
                QuotaSnapshot {
                    used_pct: Some(100.0),
                    resets_at: Some(now as i64 + 3_600_000),
                    window: Some("7d".into()),
                    status: Some("rejected".into()),
                    ..Default::default()
                },
                QuotaSnapshot {
                    window: Some("extra".into()),
                    status: Some("allowed".into()),
                    ..Default::default()
                },
            ]),
            None,
            now,
            now,
        );
        assert_eq!(a.health(now, &th), HealthState::Healthy);
    }
}
