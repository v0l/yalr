use std::time::{Duration, Instant};

use crate::metrics::{MetricsEmitter, MetricsUser};
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

struct Deferred {
    provider: String,
    model: String,
    error: ProviderError,
    user: Option<MetricsUser>,
}

pub(crate) struct RetryRounds {
    policy: TransientRetry,
    started: Instant,
    round: u32,
    pool: Vec<Candidate>,
    deferred: Vec<Deferred>,
    emitter: MetricsEmitter,
}

impl RetryRounds {
    pub(crate) fn new(policy: TransientRetry, emitter: MetricsEmitter) -> Self {
        Self {
            policy,
            started: Instant::now(),
            round: 0,
            pool: Vec::new(),
            deferred: Vec::new(),
            emitter,
        }
    }

    pub(crate) fn fail(
        &mut self,
        candidate: Candidate,
        model: &str,
        error: &ProviderError,
        user: &Option<MetricsUser>,
    ) {
        let provider = candidate.provider.name().to_string();
        if self.policy.rounds == 0 || !error.is_retryable_transient() {
            self.emitter.emit_provider_error(&provider, model, error, user.clone());
            return;
        }
        if self.round == 0 {
            self.deferred.push(Deferred {
                provider,
                model: model.to_string(),
                error: error.clone(),
                user: user.clone(),
            });
        }
        self.pool.push(candidate);
    }

    pub(crate) fn flush(&mut self) {
        for d in self.deferred.drain(..) {
            self.emitter.emit_provider_error(&d.provider, &d.model, &d.error, d.user);
        }
    }

    pub(crate) fn served(&mut self, provider: &str) {
        if self.round > 0 {
            self.emitter.record_transient_retry_success();
            self.deferred.retain(|d| d.provider != provider);
        }
        self.flush();
    }

    pub(crate) async fn next_round(&mut self, model: &str, op: &str) -> Option<Vec<Candidate>> {
        let pool = std::mem::take(&mut self.pool);
        let wait = self.policy.wait(self.round + 1);
        if pool.is_empty() || self.round >= self.policy.rounds || self.started.elapsed() + wait > self.policy.budget {
            self.flush();
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

impl Drop for RetryRounds {
    fn drop(&mut self) {
        self.flush();
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
