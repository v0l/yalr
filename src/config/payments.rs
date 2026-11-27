use crate::payments::provider::ProviderKind;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[derive(Default)]
pub struct PaymentsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub default_pricing: DefaultPricingConfig,
    pub gateway: Option<GatewaySettings>,
    #[serde(default)]
    pub provider_fee_ppm: i64,
    #[serde(default)]
    pub provider: Option<ProviderKind>,
    pub lnd: Option<LndSettings>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LndSettings {
    pub url: String,
    pub tls_cert_path: String,
    pub macaroon_path: String,
    #[serde(default = "LndSettings::default_invoice_expiry_secs")]
    pub invoice_expiry_secs: u32,
    #[serde(default = "LndSettings::default_repair_interval_secs")]
    pub repair_interval_secs: u64,
    #[serde(default = "LndSettings::default_network")]
    pub network: String,
}

impl LndSettings {
    fn default_invoice_expiry_secs() -> u32 { 3600 }
    fn default_repair_interval_secs() -> u64 { 300 }
    fn default_network() -> String { "mainnet".to_string() }

    pub fn new(url: &str, tls_cert_path: &str, macaroon_path: &str) -> Self {
        Self {
            url: url.to_string(),
            tls_cert_path: tls_cert_path.to_string(),
            macaroon_path: macaroon_path.to_string(),
            invoice_expiry_secs: Self::default_invoice_expiry_secs(),
            repair_interval_secs: Self::default_repair_interval_secs(),
            network: Self::default_network(),
        }
    }
}

fn readable_file(path: &str) -> bool {
    std::fs::File::open(path).and_then(|f| f.metadata()).is_ok_and(|m| m.is_file())
}

impl PaymentsConfig {
    pub fn selected_provider(&self) -> Result<Option<ProviderKind>, String> {
        match self.provider {
            Some(ProviderKind::Gateway) if self.gateway.is_none() => {
                Err("payments.provider is \"gateway\" but there is no payments.gateway section".into())
            }
            Some(ProviderKind::Lnd) if self.lnd.is_none() => {
                Err("payments.provider is \"lnd\" but there is no payments.lnd section (url, tls_cert_path, macaroon_path)".into())
            }
            Some(kind) => Ok(Some(kind)),
            None if self.gateway.is_some() => Ok(Some(ProviderKind::Gateway)),
            None if self.lnd.is_some() => Ok(Some(ProviderKind::Lnd)),
            None => Ok(None),
        }
    }

    pub fn validate(&self) -> Result<Option<ProviderKind>, String> {
        if !self.enabled {
            return Ok(None);
        }
        let selected = self.selected_provider()?;
        match (selected, &self.gateway, &self.lnd) {
            (Some(ProviderKind::Gateway), Some(gateway), _) if gateway.url.trim().is_empty() => {
                Err("payments.gateway.url is required".into())
            }
            (Some(ProviderKind::Lnd), _, Some(lnd)) => {
                if lnd.url.trim().is_empty() {
                    return Err("payments.lnd.url is required, for example https://127.0.0.1:10009".into());
                }
                for (key, path) in [("tls_cert_path", &lnd.tls_cert_path), ("macaroon_path", &lnd.macaroon_path)] {
                    if !readable_file(path) {
                        return Err(format!("payments.lnd.{key} {path:?} is not a readable file"));
                    }
                }
                if !["mainnet", "testnet", "signet", "regtest"].contains(&lnd.network.as_str()) {
                    return Err(format!("payments.lnd.network {:?} must be mainnet, testnet, signet or regtest", lnd.network));
                }
                Ok(selected)
            }
            _ => Ok(selected),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct GatewaySettings {
    pub url: String,
    #[serde(default = "GatewaySettings::default_currencies")]
    pub currencies: Vec<String>,
    #[serde(default = "GatewaySettings::default_min_amount")]
    pub min_amount: i64,
    #[serde(default = "GatewaySettings::default_max_amount")]
    pub max_amount: i64,
    #[serde(default = "GatewaySettings::default_reconcile_after_secs")]
    pub reconcile_after_secs: i64,
    #[serde(default = "GatewaySettings::default_cooldown_secs")]
    pub auto_reload_cooldown_secs: i64,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    #[serde(default)]
    pub sandbox: bool,
}

impl GatewaySettings {
    fn default_currencies() -> Vec<String> { vec!["sats".to_string(), "eur".to_string(), "usd".to_string()] }
    fn default_min_amount() -> i64 { 5 }
    fn default_max_amount() -> i64 { 500 }
    fn default_reconcile_after_secs() -> i64 { 120 }
    fn default_cooldown_secs() -> i64 { 3600 }
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            url: String::new(),
            currencies: Self::default_currencies(),
            min_amount: Self::default_min_amount(),
            max_amount: Self::default_max_amount(),
            reconcile_after_secs: Self::default_reconcile_after_secs(),
            auto_reload_cooldown_secs: Self::default_cooldown_secs(),
            success_url: None,
            cancel_url: None,
            sandbox: false,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DefaultPricingConfig {
    #[serde(default)]
    pub price_per_1m_input_sats: i64,
    #[serde(default)]
    pub price_per_1m_output_sats: i64,
    #[serde(default)]
    pub price_per_request_sats: i64,
    #[serde(default = "DefaultPricingConfig::default_context_window")]
    pub context_window: i32,
    #[serde(default = "DefaultPricingConfig::default_max_output_tokens")]
    pub max_output_tokens: i32,
}

impl Default for DefaultPricingConfig {
    fn default() -> Self {
        Self {
            price_per_1m_input_sats: 0,
            price_per_1m_output_sats: 0,
            price_per_request_sats: 0,
            context_window: Self::default_context_window(),
            max_output_tokens: Self::default_max_output_tokens(),
        }
    }
}

impl DefaultPricingConfig {
    fn default_context_window() -> i32 { 8192 }
    fn default_max_output_tokens() -> i32 { 4096 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> PaymentsConfig {
        config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    struct TempFile(std::path::PathBuf);

    impl TempFile {
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp_file() -> TempFile {
        let path = std::env::temp_dir().join(format!("yalr-lnd-{}", crate::db::random_hex()));
        std::fs::write(&path, b"x").unwrap();
        TempFile(path)
    }

    fn files() -> (TempFile, TempFile) {
        (temp_file(), temp_file())
    }

    #[test]
    fn the_hosted_gateway_config_still_selects_the_gateway() {
        let c = parse("enabled: true\ngateway:\n  url: https://pay.example\n");
        assert_eq!(c.validate(), Ok(Some(ProviderKind::Gateway)));
    }

    #[test]
    fn a_gateway_config_that_still_carries_an_old_lnd_section_stays_on_the_gateway() {
        let (cert, mac) = files();
        let mut c = parse("enabled: true\ngateway:\n  url: https://pay.example\n");
        c.lnd = Some(LndSettings::new("https://127.0.0.1:10009", cert.path().to_str().unwrap(), mac.path().to_str().unwrap()));
        assert_eq!(c.validate(), Ok(Some(ProviderKind::Gateway)));
    }

    #[test]
    fn an_lnd_section_alone_selects_lnd() {
        let (cert, mac) = files();
        let yaml = format!(
            "enabled: true\nlnd:\n  url: https://127.0.0.1:10009\n  tls_cert_path: {}\n  macaroon_path: {}\n",
            cert.path().display(),
            mac.path().display()
        );
        let c = parse(&yaml);
        assert_eq!(c.lnd.as_ref().map(|l| (l.invoice_expiry_secs, l.network.as_str())), Some((3600, "mainnet")));
        assert_eq!(c.validate(), Ok(Some(ProviderKind::Lnd)));
    }

    #[test]
    fn provider_lnd_wins_over_a_gateway_section() {
        let (cert, mac) = files();
        let mut c = parse("enabled: true\nprovider: lnd\ngateway:\n  url: https://pay.example\n");
        c.lnd = Some(LndSettings::new("https://127.0.0.1:10009", cert.path().to_str().unwrap(), mac.path().to_str().unwrap()));
        assert_eq!(c.validate(), Ok(Some(ProviderKind::Lnd)));
    }

    #[test]
    fn a_named_provider_without_its_section_is_a_clear_error() {
        let err = parse("enabled: true\nprovider: lnd\ngateway:\n  url: https://pay.example\n").validate().unwrap_err();
        assert!(err.contains("no payments.lnd section"), "{err}");
        let err = parse("enabled: true\nprovider: gateway\n").validate().unwrap_err();
        assert!(err.contains("no payments.gateway section"), "{err}");
    }

    #[test]
    fn an_unknown_provider_name_does_not_parse() {
        let parsed: Result<PaymentsConfig, _> = config::Config::builder()
            .add_source(config::File::from_str("enabled: true\nprovider: stripe\n", config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize();
        assert!(parsed.is_err());
    }

    #[test]
    fn lnd_with_a_missing_macaroon_is_rejected_at_start() {
        let (cert, _) = files();
        let mut c = parse("enabled: true\nprovider: lnd\n");
        c.lnd = Some(LndSettings::new("https://127.0.0.1:10009", cert.path().to_str().unwrap(), "/nonexistent/admin.macaroon"));
        let err = c.validate().unwrap_err();
        assert!(err.contains("macaroon_path") && err.contains("/nonexistent/admin.macaroon"), "{err}");
    }

    #[test]
    fn lnd_with_an_empty_url_or_bad_network_is_rejected() {
        let (cert, mac) = files();
        let mut c = parse("enabled: true\n");
        c.lnd = Some(LndSettings::new(" ", cert.path().to_str().unwrap(), mac.path().to_str().unwrap()));
        assert!(c.validate().unwrap_err().contains("payments.lnd.url"));
        let mut lnd = LndSettings::new("https://127.0.0.1:10009", cert.path().to_str().unwrap(), mac.path().to_str().unwrap());
        lnd.network = "litecoin".into();
        c.lnd = Some(lnd);
        assert!(c.validate().unwrap_err().contains("network"));
    }

    #[test]
    fn disabled_payments_or_no_provider_section_validate_to_nothing() {
        assert_eq!(parse("enabled: false\nprovider: lnd\n").validate(), Ok(None));
        assert_eq!(parse("enabled: true\n").validate(), Ok(None));
    }
}
