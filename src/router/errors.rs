use crate::providers::ProviderError;
use crate::router::RouterError;

impl RouterError {
    pub fn rate_limit_retry_ms(&self) -> Option<u64> {
        match self {
            RouterError::ProviderError(ProviderError::RateLimit { retry_after_ms, .. }) => Some(*retry_after_ms),
            _ => None,
        }
    }

    pub fn is_outage(&self) -> bool {
        matches!(self, RouterError::ProviderError(e) if e.is_outage())
    }
}

#[derive(Default)]
pub(crate) struct FailureLog(Vec<String>);

impl FailureLog {
    pub(crate) fn push(&mut self, provider: &str, error: &ProviderError) {
        self.0.push(format!("{provider}: {error}"));
    }

    pub(crate) fn warn(&self, model: &str, op: &str) {
        if !self.0.is_empty() {
            tracing::warn!(model, op, errors = %self.0.join(" | "), "All providers failed");
        }
    }
}

pub(crate) fn keep_useful(previous: Option<RouterError>, next: RouterError) -> RouterError {
    let Some(previous) = previous else {
        return next;
    };
    match (previous.rate_limit_retry_ms(), next.rate_limit_retry_ms()) {
        (Some(before), Some(now)) => {
            if now < before {
                next
            } else {
                previous
            }
        }
        (Some(_), None) => next,
        (None, Some(_)) if previous.is_outage() => previous,
        _ => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limited(ms: u64) -> RouterError {
        RouterError::ProviderError(ProviderError::RateLimit { retry_after_ms: ms, message: String::new() })
    }

    #[test]
    fn the_soonest_rate_limit_wins() {
        let kept = keep_useful(Some(limited(387_042_000)), limited(600_000));
        assert_eq!(kept.rate_limit_retry_ms(), Some(600_000));
        let kept = keep_useful(Some(limited(600_000)), limited(387_042_000));
        assert_eq!(kept.rate_limit_retry_ms(), Some(600_000));
    }

    #[test]
    fn an_outage_replaces_an_earlier_rate_limit() {
        let kept = keep_useful(Some(limited(600_000)), RouterError::ProviderError(ProviderError::Timeout));
        assert!(kept.is_outage());
    }

    #[test]
    fn an_outage_outlives_a_later_rate_limit() {
        let outage = RouterError::ProviderError(ProviderError::ServerError { message: "down".into(), status_code: Some(503) });
        let kept = keep_useful(Some(outage), limited(106_649_000));
        assert!(kept.is_outage());
        assert!(kept.rate_limit_retry_ms().is_none());
    }

    #[test]
    fn a_fatal_error_replaces_a_rate_limit() {
        let fatal = RouterError::ProviderError(ProviderError::Authentication("no".into()));
        assert!(keep_useful(Some(limited(1)), fatal).rate_limit_retry_ms().is_none());
    }
}
