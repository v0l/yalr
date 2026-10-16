use super::quota_exhausted;
use crate::providers::provider_trait::{QuotaCredit, QuotaSnapshot};

pub const EXTRA_USAGE_WINDOW: &str = "extra";

pub(super) fn is_extra_usage(q: &QuotaSnapshot) -> bool {
    q.window.as_deref() == Some(EXTRA_USAGE_WINDOW)
}

pub fn extra_usage_available(quotas: &[QuotaSnapshot]) -> bool {
    quotas.iter().any(|q| is_extra_usage(q) && !quota_exhausted(q))
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrepaidBalance {
    pub amount_minor: i64,
    pub currency: String,
    pub decimals: u32,
}

pub fn has_extra_usage(quotas: &[QuotaSnapshot]) -> bool {
    quotas.iter().any(is_extra_usage)
}

pub fn apply_prepaid_balance(quotas: &mut [QuotaSnapshot], balance: PrepaidBalance) {
    for q in quotas.iter_mut().filter(|q| is_extra_usage(q)) {
        let credit = q.credit.get_or_insert_with(|| QuotaCredit {
            currency: balance.currency.clone(),
            decimals: balance.decimals,
            used_minor: 0,
            limit_minor: None,
            balance_minor: None,
        });
        credit.balance_minor = Some(balance.amount_minor);
        if balance.amount_minor <= 0 {
            q.status = Some("rejected".to_string());
        }
    }
}

pub fn carry_credit(previous: &[QuotaSnapshot], mut next: Vec<QuotaSnapshot>) -> Vec<QuotaSnapshot> {
    let credit = previous.iter().find(|q| is_extra_usage(q)).and_then(|q| q.credit.clone());
    if let Some(credit) = credit {
        for q in next.iter_mut().filter(|q| is_extra_usage(q) && q.credit.is_none()) {
            q.credit = Some(credit.clone());
        }
    }
    next
}

pub fn blocking_quotas(quotas: &[QuotaSnapshot]) -> Vec<&QuotaSnapshot> {
    if extra_usage_available(quotas) {
        return Vec::new();
    }
    quotas
        .iter()
        .filter(|q| !is_extra_usage(q) && quota_exhausted(q))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::quota::anthropic_quotas_from_headers;
    use reqwest::header::{HeaderMap, HeaderValue};

    fn hm(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn test_carry_credit_into_header_snapshot() {
        let credit = QuotaCredit {
            currency: "EUR".into(),
            decimals: 2,
            used_minor: 5348,
            limit_minor: None,
            balance_minor: Some(476),
        };
        let previous = vec![QuotaSnapshot {
            window: Some(EXTRA_USAGE_WINDOW.into()),
            credit: Some(credit.clone()),
            ..Default::default()
        }];
        let next = carry_credit(
            &previous,
            anthropic_quotas_from_headers(&hm(&[("anthropic-ratelimit-unified-overage-status", "allowed")])),
        );
        assert_eq!(next[0].credit, Some(credit));
    }

    #[test]
    fn test_prepaid_balance_on_extra_window() {
        let mut qs = anthropic_quotas_from_headers(&hm(&[
            ("anthropic-ratelimit-unified-7d-status", "rejected"),
            ("anthropic-ratelimit-unified-overage-status", "allowed"),
        ]));
        let balance = |amount_minor| PrepaidBalance { amount_minor, currency: "EUR".into(), decimals: 2 };
        apply_prepaid_balance(&mut qs, balance(476));
        let extra = qs.iter().find(|q| is_extra_usage(q)).unwrap();
        assert_eq!(extra.credit.as_ref().unwrap().balance_minor, Some(476));
        assert!(blocking_quotas(&qs).is_empty());

        apply_prepaid_balance(&mut qs, balance(0));
        assert_eq!(blocking_quotas(&qs).len(), 1);
    }
}
