use crate::providers::ProviderError;
use crate::router::RouterError;

impl RouterError {
    pub fn rate_limit_retry_ms(&self) -> Option<u64> {
        match self {
            RouterError::ProviderError(ProviderError::RateLimit { retry_after_ms, .. }) => Some(*retry_after_ms),
            _ => None,
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
        (Some(_), None) => match &next {
            RouterError::ProviderError(e) if e.is_transient() => previous,
            _ => next,
        },
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
    fn a_rate_limit_outlives_a_later_transient_failure() {
        let kept = keep_useful(Some(limited(600_000)), RouterError::ProviderError(ProviderError::Timeout));
        assert_eq!(kept.rate_limit_retry_ms(), Some(600_000));
    }

    #[test]
    fn a_fatal_error_replaces_a_rate_limit() {
        let fatal = RouterError::ProviderError(ProviderError::Authentication("no".into()));
        assert!(keep_useful(Some(limited(1)), fatal).rate_limit_retry_ms().is_none());
    }
}
