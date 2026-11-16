use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::db::payouts::PayoutRules;
use crate::db::{milli_per_unit, SATS};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PayoutsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_min_fiat")]
    pub min_fiat: i64,
    #[serde(default = "default_min_sats")]
    pub min_sats: i64,
    #[serde(default = "default_cap_fiat")]
    pub daily_cap_fiat: i64,
    #[serde(default = "default_cap_sats")]
    pub daily_cap_sats: i64,
    #[serde(default = "default_hold_days")]
    pub hold_days: i64,
    #[serde(default = "default_approval_fiat")]
    pub approval_threshold_fiat: i64,
    #[serde(default = "default_approval_sats")]
    pub approval_threshold_sats: i64,
    #[serde(default)]
    pub fee_ppm: i64,
    #[serde(default)]
    pub fee_flat_fiat: i64,
    #[serde(default)]
    pub fee_flat_sats: i64,
    #[serde(default = "default_ln_fee_ppm")]
    pub lightning_max_fee_ppm: i64,
    pub revolut: Option<RevolutBusinessConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RevolutBusinessConfig {
    #[serde(default = "default_revolut_url")]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub webhook_secret: String,
    #[serde(default)]
    pub account_ids: HashMap<String, String>,
}

fn default_revolut_url() -> String {
    "https://b2b.revolut.com/api/1.0".to_string()
}
fn default_min_fiat() -> i64 {
    10
}
fn default_min_sats() -> i64 {
    5_000
}
fn default_cap_fiat() -> i64 {
    1_000
}
fn default_cap_sats() -> i64 {
    5_000_000
}
fn default_hold_days() -> i64 {
    7
}
fn default_approval_fiat() -> i64 {
    250
}
fn default_approval_sats() -> i64 {
    250_000
}
fn default_ln_fee_ppm() -> i64 {
    10_000
}

impl Default for PayoutsConfig {
    fn default() -> Self {
        serde_json::from_str("{}").expect("every field has a default")
    }
}

impl PayoutsConfig {
    pub fn rules(&self, currency: &str) -> PayoutRules {
        let unit = milli_per_unit(currency);
        let sats = currency == SATS;
        let (min, cap, approval, flat) = if sats {
            (self.min_sats, self.daily_cap_sats, self.approval_threshold_sats, self.fee_flat_sats)
        } else {
            (self.min_fiat, self.daily_cap_fiat, self.approval_threshold_fiat, self.fee_flat_fiat)
        };
        PayoutRules {
            min_amount: min * unit,
            daily_cap: cap * unit,
            hold_days: self.hold_days,
            approval_threshold: approval * unit,
            fee_ppm: self.fee_ppm,
            fee_flat: flat * unit,
            granularity: 1_000,
        }
    }

    pub fn revolut_account(&self, currency: &str) -> Option<&str> {
        self.revolut.as_ref()?.account_ids.get(currency).map(String::as_str)
    }

    pub fn methods_for(&self, currency: &str) -> Vec<&'static str> {
        use crate::db::payouts::{LIGHTNING, REVOLUT_LINK};
        if currency == SATS {
            vec![LIGHTNING]
        } else if self.revolut_account(currency).is_some() {
            vec![REVOLUT_LINK]
        } else {
            vec![]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_ten_units_and_five_thousand_sats_in_milli_units() {
        let c = PayoutsConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.rules("eur").min_amount, 1_000_000);
        assert_eq!(c.rules("usd").min_amount, 1_000_000);
        assert_eq!(c.rules("sats").min_amount, 5_000_000);
        assert_eq!(c.rules("eur").hold_days, 7);
    }

    #[test]
    fn methods_follow_currency_and_configured_accounts() {
        let yaml = "enabled: true\nrevolut:\n  api_key: k\n  account_ids:\n    eur: acc\n";
        let c: PayoutsConfig = serde_yaml_or_json(yaml);
        assert_eq!(c.methods_for("sats"), vec!["lightning"]);
        assert_eq!(c.methods_for("eur"), vec!["revolut_link"]);
        assert!(c.methods_for("usd").is_empty());
    }

    fn serde_yaml_or_json(yaml: &str) -> PayoutsConfig {
        ::config::Config::builder()
            .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }
}
