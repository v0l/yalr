use std::time::{Duration, Instant};

use crate::metrics::{MetricsEmitter, MetricsUser};
use crate::providers::attribution::{attribute_timeout, AttemptInfo, FailureAttribution};
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
    attribution: FailureAttribution,
    class: String,
}

pub(crate) struct RetryRounds {
    policy: TransientRetry,
    started: Instant,
    round: u32,
    pool: Vec<Candidate>,
    deferred: Vec<Deferred>,
    emitter: MetricsEmitter,
    prompt_tokens: Option<u32>,
    mark: Instant,
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
            prompt_tokens: None,
            mark: Instant::now(),
        }
    }

    pub(crate) fn with_request_bytes(mut self, bytes: usize) -> Self {
        self.prompt_tokens = Some((bytes / 4).min(u32::MAX as usize) as u32);
        self
    }

    fn attribute(&self, provider: &str, error: &ProviderError) -> FailureAttribution {
        if matches!(error, ProviderError::Timeout) {
            let info = AttemptInfo {
                prompt_tokens: self.prompt_tokens,
                waited_ms: Some(self.mark.elapsed().as_millis() as u64),
                progressed: false,
            };
            return attribute_timeout(&info, self.emitter.prefill_rate(provider));
        }
        error.attribution()
    }

    pub(crate) fn fail(
        &mut self,
        candidate: Candidate,
        model: &str,
        error: &ProviderError,
        user: &Option<MetricsUser>,
    ) {
        let provider = candidate.provider.name().to_string();
        let attribution = self.attribute(&provider, error);
        self.mark = Instant::now();
        if self.policy.rounds == 0 || !error.is_retryable_transient() {
            self.emitter.emit_attributed_error(&provider, model, error, attribution, user.clone());
            return;
        }
        if self.round == 0 {
            self.deferred.push(Deferred {
                provider,
                model: model.to_string(),
                error: error.clone(),
                user: user.clone(),
                attribution,
                class: error.failure_class(),
            });
        }
        self.pool.push(candidate);
    }

    pub(crate) fn flush(&mut self) {
        let deferred = std::mem::take(&mut self.deferred);
        let agreed: Vec<bool> = deferred
            .iter()
            .map(|d| {
                d.attribution == FailureAttribution::Unknown
                    && deferred.iter().any(|o| o.provider != d.provider && o.class == d.class)
            })
            .collect();
        for (d, agreed) in deferred.into_iter().zip(agreed) {
            let attribution = if agreed { FailureAttribution::Request } else { d.attribution };
            self.emitter.emit_attributed_error(&d.provider, &d.model, &d.error, attribution, d.user);
        }
    }

    fn charge_to_providers(&mut self) {
        for d in self.deferred.drain(..) {
            self.emitter.emit_attributed_error(&d.provider, &d.model, &d.error, FailureAttribution::Provider, d.user);
        }
    }

    pub(crate) fn served(&mut self, provider: &str) {
        if self.round > 0 {
            self.emitter.record_transient_retry_success();
            self.deferred.retain(|d| d.provider != provider);
        }
        self.charge_to_providers();
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
