use std::time::{Duration, Instant};

use crate::metrics::MetricsEmitter;
use crate::providers::ProviderError;
use crate::router::engine::Candidate;

#[derive(Clone, Copy, Debug)]
pub struct TransientRetry {
    pub rounds: u32,
    pub budget: Duration,
    pub base: Duration,
}

impl Default for TransientRetry {
    fn default() -> Self {
        Self {
            rounds: 3,
            budget: Duration::from_millis(15_000),
            base: Duration::from_secs(1),
        }
    }
}

impl TransientRetry {
    pub fn new(rounds: u32, budget_ms: u64) -> Self {
        Self {
            rounds,
            budget: Duration::from_millis(budget_ms),
            ..Self::default()
        }
    }

    fn wait(&self, round: u32) -> Duration {
        let nominal = self.base.saturating_mul(1 << (round - 1).min(10));
        nominal.mul_f64(0.8 + rand::random::<f64>() * 0.4)
    }
}

pub(crate) struct RetryRounds {
    policy: TransientRetry,
    started: Instant,
    round: u32,
    pool: Vec<Candidate>,
    emitter: MetricsEmitter,
}

impl RetryRounds {
    pub(crate) fn new(policy: TransientRetry, emitter: MetricsEmitter) -> Self {
        Self {
            policy,
            started: Instant::now(),
            round: 0,
            pool: Vec::new(),
            emitter,
        }
    }

    pub(crate) fn first_round(&self) -> bool {
        self.round == 0
    }

    pub(crate) fn keep(&mut self, candidate: Candidate, error: &ProviderError) {
        if error.is_retryable_transient() {
            self.pool.push(candidate);
        }
    }

    pub(crate) fn served(&self) {
        if self.round > 0 {
            self.emitter.record_transient_retry_success();
        }
    }

    pub(crate) async fn next_round(&mut self, model: &str, op: &str) -> Option<Vec<Candidate>> {
        let pool = std::mem::take(&mut self.pool);
        if pool.is_empty() || self.round >= self.policy.rounds {
            return None;
        }
        let wait = self.policy.wait(self.round + 1);
        if self.started.elapsed() + wait > self.policy.budget {
            return None;
        }
        self.round += 1;
        let providers: Vec<&str> = pool.iter().map(|c| c.provider.name()).collect();
        tracing::info!(
            model,
            op,
            round = self.round,
            wait_ms = wait.as_millis() as u64,
            provider = %providers.join(","),
            "retrying after transient error"
        );
        self.emitter.record_transient_retry();
        tokio::time::sleep(wait).await;
        Some(pool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_each_round_within_jitter() {
        let policy = TransientRetry::default();
        for (round, nominal) in [(1u32, 1000u64), (2, 2000), (3, 4000)] {
            let wait = policy.wait(round).as_millis() as u64;
            assert!((nominal * 8 / 10..=nominal * 12 / 10).contains(&wait), "{round}: {wait}");
        }
    }
}
