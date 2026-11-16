mod charge;
mod runner;

use crate::config::RevolutSettings;
use crate::crypto::SecretBox;
use crate::db::fiat_orders::{FiatOrder, NewFiatOrder, CANCELLED, FAILED, KIND_AUTO_RELOAD, KIND_TOPUP};
use crate::db::payment_methods::NewPaymentMethod;
use crate::db::Database;
use payments_rs::fiat::{
    RevolutApi, RevolutConfig, RevolutCustomer, RevolutOrder, RevolutOrderState, RevolutSavedPaymentMethod,
};
use std::sync::Arc;

pub use runner::spawn_runner;

const MILLI_PER_MINOR: i64 = 1_000;

#[derive(Clone)]
pub struct RevolutSecrets {
    pub api_key: String,
    pub webhook_secret: String,
    pub encryption_key_hex: String,
}

impl RevolutSecrets {
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Some(Self {
            api_key: var("REVOLUT_API_KEY")?,
            webhook_secret: var("REVOLUT_WEBHOOK_SECRET")?,
            encryption_key_hex: var("YALR_ENCRYPTION_KEY")?,
        })
    }
}

#[derive(Debug)]
pub enum RevolutError {
    Invalid(String),
    Upstream(String),
    Database(sqlx::Error),
}

impl std::fmt::Display for RevolutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RevolutError::Invalid(m) | RevolutError::Upstream(m) => f.write_str(m),
            RevolutError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for RevolutError {}

impl From<sqlx::Error> for RevolutError {
    fn from(e: sqlx::Error) -> Self {
        RevolutError::Database(e)
    }
}

fn upstream(e: impl std::fmt::Display) -> RevolutError {
    RevolutError::Upstream(e.to_string())
}

pub struct CreatedOrder {
    pub order: FiatOrder,
    pub token: String,
}

pub struct RevolutService {
    pub(crate) api: RevolutApi,
    pub(crate) http: reqwest::Client,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) api_version: String,
    pub(crate) webhook_secret: String,
    pub(crate) secrets: SecretBox,
    pub(crate) db: Arc<Database>,
    pub settings: RevolutSettings,
}

pub fn minor_units(amount: f64) -> Option<i64> {
    let minor = (amount * 100.0).round();
    ((amount * 100.0 - minor).abs() < 1e-6 && minor > 0.0 && minor < 1e12).then_some(minor as i64)
}

impl RevolutService {
    pub fn new(settings: RevolutSettings, secrets: RevolutSecrets, db: Arc<Database>) -> Result<Self, RevolutError> {
        let base_url = settings.url.clone().unwrap_or_else(|| {
            if settings.sandbox { "https://sandbox-merchant.revolut.com" } else { "https://merchant.revolut.com" }.to_string()
        });
        let api = RevolutApi::new(RevolutConfig {
            url: Some(base_url.clone()),
            api_version: settings.api_version.clone(),
            token: secrets.api_key.clone(),
            public_key: String::new(),
        })
        .map_err(upstream)?;
        let secret_box = SecretBox::from_hex(&secrets.encryption_key_hex).map_err(upstream)?;
        Ok(Self {
            api,
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: secrets.api_key,
            api_version: settings.api_version.clone(),
            webhook_secret: secrets.webhook_secret,
            secrets: secret_box,
            db,
            settings,
        })
    }

    pub fn webhook_secret(&self) -> &str {
        &self.webhook_secret
    }

    pub fn mode(&self) -> &'static str {
        if self.settings.sandbox { "sandbox" } else { "prod" }
    }

    pub fn currency_allowed(&self, currency: &str) -> bool {
        self.settings.currencies.iter().any(|c| c == currency)
    }

    pub fn check_amount(&self, amount: f64) -> Result<i64, RevolutError> {
        let minor = minor_units(amount).ok_or_else(|| RevolutError::Invalid("Amount must be a positive number with at most 2 decimals".into()))?;
        let (min, max) = (self.settings.min_amount * 100, self.settings.max_amount * 100);
        if minor < min || minor > max {
            return Err(RevolutError::Invalid(format!(
                "Amount must be between {} and {}",
                self.settings.min_amount, self.settings.max_amount
            )));
        }
        Ok(minor)
    }

    pub(crate) fn customer_for(&self, stored: Option<String>, email: Option<String>) -> Result<Option<RevolutCustomer>, RevolutError> {
        if let Some(sealed) = stored {
            let id = self.secrets.decrypt(&sealed).map_err(upstream)?;
            return Ok(Some(RevolutCustomer { id: Some(id), ..Default::default() }));
        }
        Ok(email.map(|email| RevolutCustomer { email: Some(email), ..Default::default() }))
    }

    pub async fn create_topup(
        &self,
        user_id: i64,
        currency: &str,
        minor: i64,
        save_card: bool,
        email: Option<String>,
    ) -> Result<CreatedOrder, RevolutError> {
        let stored = self.db.user_customer_ids_encrypted(user_id).await?;
        let email = email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
        if save_card && stored.is_none() && email.is_none() {
            return Err(RevolutError::Invalid("An email address is needed to save a card".into()));
        }
        let customer = if save_card { self.customer_for(stored, email)? } else { None };
        let order = self
            .db
            .create_fiat_order(NewFiatOrder {
                user_id,
                kind: KIND_TOPUP,
                currency,
                amount_milli: minor * MILLI_PER_MINOR,
                save_card,
            })
            .await?;
        match self.send_order(&order, minor, customer, save_card).await {
            Ok(created) => Ok(CreatedOrder { order: self.db.fiat_order(&order.public_id).await?.unwrap_or(order), token: created.token }),
            Err(e) => {
                self.db.fail_unsent_fiat_order(order.id, "processor rejected the order").await?;
                Err(e)
            }
        }
    }

    pub(crate) async fn send_order(
        &self,
        order: &FiatOrder,
        minor: i64,
        customer: Option<RevolutCustomer>,
        save_card: bool,
    ) -> Result<RevolutOrder, RevolutError> {
        let currency = order.currency.parse::<payments_rs::currency::Currency>().map_err(|_| RevolutError::Invalid("Unsupported currency".into()))?;
        let amount = payments_rs::currency::CurrencyAmount::from_u64(currency, minor as u64);
        let created = self
            .api
            .create_order_ext(
                amount,
                Some(format!("YALR balance top-up {}", order.public_id)),
                None,
                customer,
                save_card.then(|| "merchant".to_string()),
            )
            .await
            .map_err(upstream)?;
        self.db.set_fiat_order_external(order.id, &created.id).await?;
        Ok(created)
    }

    pub async fn reconcile(&self, external_id: &str) -> Result<Option<FiatOrder>, RevolutError> {
        let Some(local) = self.db.fiat_order_by_external(external_id).await? else {
            return Ok(None);
        };
        let remote = self.api.get_order(external_id).await.map_err(upstream)?;
        self.apply(&local, &remote).await
    }

    pub(crate) async fn apply(&self, local: &FiatOrder, remote: &RevolutOrder) -> Result<Option<FiatOrder>, RevolutError> {
        let outcome = match remote.state {
            RevolutOrderState::Completed => {
                if !self.matches(local, remote) {
                    tracing::error!(order = %local.public_id, "Revolut order amount or currency differs from ours, not crediting");
                    self.db.close_fiat_order(&remote.id, FAILED, Some("amount mismatch")).await?
                } else {
                    let done = self.db.complete_fiat_order(&remote.id).await?;
                    if let Some(done) = done.as_ref() {
                        if done.save_card {
                            if let Err(e) = self.save_cards(done.user_id, remote).await {
                                tracing::warn!(order = %done.public_id, error = %e, "Could not save the card after top-up");
                            }
                        }
                    }
                    done
                }
            }
            RevolutOrderState::Cancelled => self.db.close_fiat_order(&remote.id, CANCELLED, None).await?,
            RevolutOrderState::Failed => self.db.close_fiat_order(&remote.id, FAILED, Some("payment failed")).await?,
            _ => None,
        };
        if let Some(closed) = outcome.as_ref().filter(|o| o.kind == KIND_AUTO_RELOAD) {
            self.db.note_auto_reload_outcome(closed.user_id, &closed.currency, closed.status == crate::db::fiat_orders::COMPLETED).await?;
        }
        Ok(outcome)
    }

    fn matches(&self, local: &FiatOrder, remote: &RevolutOrder) -> bool {
        remote.currency.eq_ignore_ascii_case(&local.currency) && remote.amount as i64 * MILLI_PER_MINOR == local.amount_milli
    }

    async fn save_cards(&self, user_id: i64, remote: &RevolutOrder) -> Result<(), RevolutError> {
        let Some(customer_id) = remote.customer_id() else { return Ok(()) };
        let methods = self.api.get_customer_payment_methods(&customer_id, true).await.map_err(upstream)?;
        let sealed = self.secrets.encrypt(&customer_id);
        for method in methods {
            self.store_method(user_id, &sealed, &method).await?;
        }
        Ok(())
    }

    async fn store_method(&self, user_id: i64, sealed_customer: &str, method: &RevolutSavedPaymentMethod) -> Result<(), RevolutError> {
        let details = method.method_details.as_ref();
        self.db
            .add_payment_method(NewPaymentMethod {
                user_id,
                external_customer_id_encrypted: sealed_customer,
                external_id: &method.id,
                card_brand: details.and_then(|d| d.brand.as_deref()),
                card_last_four: details.and_then(|d| d.last4.as_deref()),
                exp_month: details.and_then(|d| d.expiry_month),
                exp_year: details.and_then(|d| d.expiry_year),
            })
            .await?;
        Ok(())
    }

    pub async fn remove_card(&self, user_id: i64, id: i64) -> Result<bool, RevolutError> {
        let Some(method) = self.db.payment_method(user_id, id).await? else { return Ok(false) };
        if let Ok(customer) = self.secrets.decrypt(&method.external_customer_id) {
            let url = format!("{}/api/customers/{}/payment-methods/{}", self.base_url, customer, method.external_id);
            let result = self
                .http
                .delete(url)
                .bearer_auth(&self.api_key)
                .header("Revolut-Api-Version", &self.api_version)
                .send()
                .await;
            match result {
                Ok(r) if r.status().is_success() || r.status() == reqwest::StatusCode::NOT_FOUND => {}
                Ok(r) => tracing::warn!(status = %r.status(), "Revolut refused to delete the saved card, removing it locally"),
                Err(e) => tracing::warn!(error = %e, "Revolut card delete failed, removing it locally"),
            }
        }
        Ok(self.db.delete_payment_method(user_id, id).await?)
    }
}

#[cfg(test)]
mod sandbox_tests {
    use super::*;

    fn local_token() -> Option<String> {
        let path = std::env::var("YALR_REVOLUT_SANDBOX_CONFIG").ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        text.lines().find_map(|l| l.trim().strip_prefix("token:").map(|t| t.trim().trim_matches('"').to_string()))
    }

    #[tokio::test]
    #[ignore = "needs YALR_REVOLUT_SANDBOX_CONFIG pointing at a yaml with a sandbox token"]
    async fn a_real_sandbox_order_is_created_and_stays_pending_until_paid() {
        let token = local_token().expect("sandbox token");
        let db = Arc::new(Database::test().await);
        let user = db.add_test_user("u", false).await;
        let settings = RevolutSettings { sandbox: true, ..Default::default() };
        let secrets = RevolutSecrets { api_key: token, webhook_secret: "unused".into(), encryption_key_hex: "00".repeat(32) };
        let service = RevolutService::new(settings, secrets, db.clone()).unwrap();
        let created = service.create_topup(user.id, "eur", 500, false, None).await.unwrap();
        assert!(!created.token.is_empty());
        let reconciled = service.reconcile(created.order.external_id.as_deref().unwrap()).await.unwrap();
        assert!(reconciled.is_none());
        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 0);
    }
}
