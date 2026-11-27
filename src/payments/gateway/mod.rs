mod cards;
#[cfg(test)]
pub mod fake;
mod provider;
mod runner;
mod settle;

use crate::config::GatewaySettings;
use crate::db::gateway_payments::{GatewayPayment, NewGatewayPayment, RemotePayment};
use crate::db::{random_hex, Database, SATS};
use std::sync::Arc;

pub use crate::payments::provider::TopupRequest;
pub use payment_gateway_client::{Payment as RemotePaymentView, SavedMethod as RemoteSavedMethod};
pub use runner::spawn_runner;

use payment_gateway_client::{CreateAttempt, CreatePayment, Gateway};

const MILLI_PER_MINOR: i64 = 1_000;
const PAYMENT_TTL_SECS: u32 = 3600;
pub const SATS_MIN: i64 = 1;
pub const SATS_MAX: i64 = 10_000_000;

#[derive(Clone)]
pub struct GatewaySecrets {
    pub api_key: String,
    pub webhook_secret: String,
}

impl GatewaySecrets {
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Some(Self { api_key: var("GATEWAY_API_KEY")?, webhook_secret: var("GATEWAY_WEBHOOK_SECRET")? })
    }
}

pub type GatewayError = crate::payments::provider::PaymentError;

impl From<payment_gateway_client::Error> for GatewayError {
    fn from(e: payment_gateway_client::Error) -> Self {
        match e.status() {
            Some(status) if (400..500).contains(&status) && status != 429 => GatewayError::Rejected(e.to_string()),
            _ => GatewayError::Upstream(e.to_string()),
        }
    }
}

pub(crate) fn lightning_details(payment: &RemotePaymentView) -> (Option<String>, Option<String>) {
    let details = payment.attempts.iter().find(|a| a.method == "lightning").map(|a| &a.details);
    let text = |key: &str| details.and_then(|d| d.get(key)).and_then(|v| v.as_str()).map(str::to_string);
    (text("bolt11"), text("payment_hash"))
}

pub(crate) fn checkout_url(payment: &RemotePaymentView) -> Option<String> {
    payment.attempts.iter().find_map(|a| a.details.get("checkout_url")).and_then(|v| v.as_str()).map(str::to_string)
}

pub(crate) fn expires(payment: &RemotePaymentView) -> Option<crate::db::Timestamp> {
    payment.expires.as_deref().and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok()).map(|t| t.with_timezone(&chrono::Utc))
}

pub struct GatewayService {
    pub(crate) client: Gateway,
    pub(crate) webhook_secret: String,
    pub(crate) db: Arc<Database>,
    pub settings: GatewaySettings,
    api_key: String,
    http: reqwest::Client,
    usd_rate: tokio::sync::Mutex<Option<(std::time::Instant, f64)>>,
}

const RATE_TTL: std::time::Duration = std::time::Duration::from_secs(300);
const RATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(serde::Deserialize)]
struct RatesResponse {
    btc: std::collections::HashMap<String, f64>,
}

pub fn check_sats(amount: f64) -> Result<i64, GatewayError> {
    let sats = amount.round() as i64;
    if (amount - sats as f64).abs() > 1e-9 || !(SATS_MIN..=SATS_MAX).contains(&sats) {
        return Err(GatewayError::Invalid(format!("Amount must be a whole number of sats between {SATS_MIN} and {SATS_MAX}")));
    }
    Ok(sats * MILLI_PER_MINOR)
}

pub fn minor_units(amount: f64) -> Option<i64> {
    let minor = (amount * 100.0).round();
    ((amount * 100.0 - minor).abs() < 1e-6 && minor > 0.0 && minor < 1e12).then_some(minor as i64)
}

pub fn remote_currency(currency: &str) -> String {
    if currency == SATS { "BTC".to_string() } else { currency.to_ascii_uppercase() }
}

pub fn local_currency(remote: &str) -> String {
    if remote.eq_ignore_ascii_case("BTC") { SATS.to_string() } else { remote.to_ascii_lowercase() }
}

pub fn remote_amount(currency: &str, amount_milli: i64) -> i64 {
    if currency == SATS { amount_milli } else { amount_milli / MILLI_PER_MINOR }
}

impl GatewayService {
    pub fn new(settings: GatewaySettings, secrets: GatewaySecrets, db: Arc<Database>) -> Result<Self, GatewayError> {
        if settings.url.trim().is_empty() {
            return Err(GatewayError::Invalid("payments.gateway.url is required".into()));
        }
        Ok(Self {
            client: Gateway::new(&settings.url, &secrets.api_key),
            webhook_secret: secrets.webhook_secret,
            db,
            settings,
            api_key: secrets.api_key,
            http: reqwest::Client::new(),
            usd_rate: Default::default(),
        })
    }

    pub async fn btc_usd(&self) -> Option<f64> {
        let mut cached = self.usd_rate.lock().await;
        if let Some((at, rate)) = *cached {
            if at.elapsed() < RATE_TTL {
                return Some(rate);
            }
        }
        match self.fetch_btc_usd().await {
            Some(rate) => {
                *cached = Some((std::time::Instant::now(), rate));
                Some(rate)
            }
            None => None,
        }
    }

    async fn fetch_btc_usd(&self) -> Option<f64> {
        let url = format!("{}/v1/rates", self.settings.url.trim_end_matches('/'));
        let response = self.http.get(url).bearer_auth(&self.api_key).timeout(RATE_TIMEOUT).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let rates: RatesResponse = response.json().await.ok()?;
        rates.btc.get("USD").copied().filter(|r| r.is_finite() && *r > 0.0)
    }

    pub fn mode(&self) -> &'static str {
        if self.settings.sandbox { "sandbox" } else { "prod" }
    }

    pub fn currency_allowed(&self, currency: &str) -> bool {
        self.settings.currencies.iter().any(|c| c == currency)
    }

    pub fn customer_ref(user_id: i64) -> String {
        format!("user-{user_id}")
    }

    pub fn check_amount(&self, currency: &str, amount: f64) -> Result<i64, GatewayError> {
        if currency == SATS {
            return check_sats(amount);
        }
        let minor = minor_units(amount)
            .ok_or_else(|| GatewayError::Invalid("Amount must be a positive number with at most 2 decimals".into()))?;
        let (min, max) = (self.settings.min_amount * 100, self.settings.max_amount * 100);
        if minor < min || minor > max {
            return Err(GatewayError::Invalid(format!(
                "Amount must be between {} and {}",
                self.settings.min_amount, self.settings.max_amount
            )));
        }
        Ok(minor * MILLI_PER_MINOR)
    }

    pub async fn create_topup(&self, request: TopupRequest<'_>) -> Result<GatewayPayment, GatewayError> {
        let public_id = request.public_id.map_or_else(random_hex, str::to_string);
        let order = self
            .db
            .create_gateway_payment(NewGatewayPayment {
                public_id: &public_id,
                user_id: request.user_id,
                kind: request.kind,
                purpose: request.purpose,
                currency: request.currency,
                amount_milli: request.amount_milli,
            })
            .await?;
        match self.send_payment(&order, &request).await {
            Ok(()) => Ok(self.db.gateway_payment(&public_id).await?.unwrap_or(order)),
            Err(e) => {
                self.db.fail_unsent_gateway_payment(order.id, "gateway rejected the payment").await?;
                Err(e)
            }
        }
    }

    async fn send_payment(&self, order: &GatewayPayment, request: &TopupRequest<'_>) -> Result<(), GatewayError> {
        let body = CreatePayment {
            reference: Some(order.public_id.clone()),
            amount: remote_amount(&order.currency, order.amount_milli) as u64,
            currency: remote_currency(&order.currency),
            expires_in: Some(PAYMENT_TTL_SECS as i64),
            customer_ref: Some(Self::customer_ref(order.user_id)),
            customer_email: request.email.map(str::to_string),
            description: Some(format!("YALR balance top-up {}", order.public_id)),
            success_url: self.settings.success_url.clone(),
            cancel_url: self.settings.cancel_url.clone(),
            methods: request.lightning.then(|| vec!["lightning".to_string()]),
            ..Default::default()
        };
        let created = self.client.create_payment_with_key(&body, &order.public_id).await?;
        let view = if request.lightning { self.client.create_attempt(&created.id, &CreateAttempt::new("lightning")).await? } else { created };
        let (bolt11, payment_hash) = lightning_details(&view);
        self.db
            .attach_remote_payment(
                order.id,
                RemotePayment {
                    gateway_payment_id: &view.id,
                    pay_url: Some(&view.pay_url),
                    bolt11: bolt11.as_deref(),
                    payment_hash: payment_hash.as_deref(),
                    expires_at: expires(&view),
                },
            )
            .await?;
        Ok(())
    }
}
