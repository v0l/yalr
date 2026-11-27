use std::sync::Arc;

use crate::db::payouts::{
    NewPayout, PayoutError, PayoutRow, HELD, LIGHTNING, REQUESTED, REVOLUT_LINK, SENT,
};
use crate::db::Database;
use crate::state::AppState;

use super::config::PayoutsConfig;
use super::lightning::{resolve_destination, validate_destination, InvoicePayer, LightningPayoutError};
use super::revolut::{LinkState, NewLink, PayoutLink, RevolutBusiness, RevolutError};

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("payouts are not enabled")]
    Disabled,
    #[error("{0} payouts are not available for this currency")]
    MethodUnavailable(String),
    #[error("{0}")]
    Destination(String),
    #[error("{0:?}")]
    Rejected(PayoutError),
}

impl From<PayoutError> for RequestError {
    fn from(e: PayoutError) -> Self {
        Self::Rejected(e)
    }
}

impl From<LightningPayoutError> for RequestError {
    fn from(e: LightningPayoutError) -> Self {
        Self::Destination(e.to_string())
    }
}

pub struct PayoutRequest<'a> {
    pub user_id: i64,
    pub currency: &'a str,
    pub amount: i64,
    pub method: &'a str,
    pub destination: &'a str,
}

#[derive(Clone)]
pub struct PayoutService {
    db: Arc<Database>,
    config: PayoutsConfig,
    lightning: Option<Arc<dyn InvoicePayer>>,
    revolut: Option<RevolutBusiness>,
    http: reqwest::Client,
}

impl PayoutService {
    pub fn new(
        db: Arc<Database>,
        config: PayoutsConfig,
        lightning: Option<Arc<dyn InvoicePayer>>,
        revolut: Option<RevolutBusiness>,
    ) -> Self {
        Self { db, config, lightning, revolut, http: crate::providers::egress::Egress::default().http() }
    }

    pub fn from_state(state: &AppState) -> Self {
        let config = super::settings::get();
        let lightning = state
            .payments_state
            .as_ref()
            .and_then(|p| p.lnd.clone())
            .map(|l| l as Arc<dyn InvoicePayer>);
        let revolut = config.revolut.as_ref().filter(|r| !r.api_key.is_empty()).map(RevolutBusiness::new);
        Self::new(state.db.clone(), config, lightning, revolut)
    }

    pub fn config(&self) -> &PayoutsConfig {
        &self.config
    }

    pub async fn request(&self, req: PayoutRequest<'_>) -> Result<PayoutRow, RequestError> {
        if !self.config.enabled {
            return Err(RequestError::Disabled);
        }
        if !self.config.methods_for(req.currency).contains(&req.method) {
            return Err(RequestError::MethodUnavailable(req.method.to_string()));
        }
        let rules = self.config.rules(req.currency);
        let fee = rules.fee_for(req.amount);
        let mut external_id = None;
        let destination = req.destination.trim();
        let resolved_hash;
        if req.method == LIGHTNING {
            validate_destination(destination)?;
            if super::lightning::is_bolt11(destination) {
                let net = (req.amount - fee).max(0) as u64;
                resolved_hash = resolve_destination(&self.http, destination, net).await?.payment_hash;
                external_id = Some(resolved_hash.as_str());
            }
        } else if destination.is_empty() || destination.len() > 120 {
            return Err(RequestError::Destination("recipient name is required".into()));
        }
        let new = NewPayout {
            user_id: req.user_id,
            currency: req.currency,
            amount: req.amount,
            method: req.method,
            destination,
            external_id,
        };
        let row = self.db.request_payout(new, &rules).await?;
        if row.status == HELD {
            self.process(row.id).await;
        }
        Ok(self.db.get_payout(row.id).await.ok().flatten().unwrap_or(row))
    }

    pub async fn approve(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        let row = self.db.approve_payout(id).await?;
        self.process(id).await;
        Ok(self.db.get_payout(id).await.ok().flatten().unwrap_or(row))
    }

    pub async fn settle(&self, id: i64, external_id: &str) -> Result<PayoutRow, PayoutError> {
        let row = self.db.get_payout(id).await?.ok_or(PayoutError::NotFound)?;
        if row.status == HELD {
            self.db.mark_payout_sent(id, external_id, None).await?;
        }
        self.db.complete_payout(id).await
    }

    pub async fn retry(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        self.db.retry_payout(id).await
    }

    pub async fn cancel(&self, id: i64, owner: Option<i64>) -> Result<PayoutRow, PayoutError> {
        let row = self.db.get_payout(id).await?.ok_or(PayoutError::NotFound)?;
        if owner.is_some_and(|o| o != row.user_id) {
            return Err(PayoutError::NotFound);
        }
        match row.status.as_str() {
            REQUESTED => self.db.reject_payout(id, "cancelled by user").await,
            HELD => self.db.cancel_payout(id).await,
            SENT if row.method == REVOLUT_LINK => {
                let (Some(revolut), Some(link_id)) = (&self.revolut, row.external_id.as_deref()) else {
                    return Err(PayoutError::WrongState);
                };
                revolut.cancel_payout_link(link_id).await.map_err(|e| PayoutError::Db(e.to_string()))?;
                self.db.cancel_payout(id).await
            }
            _ => Err(PayoutError::WrongState),
        }
    }

    pub async fn process(&self, id: i64) {
        let Ok(Some(row)) = self.db.get_payout(id).await else {
            return;
        };
        if row.status != HELD {
            return;
        }
        match row.method.as_str() {
            LIGHTNING => self.send_lightning(row).await,
            REVOLUT_LINK => self.send_revolut(row).await,
            other => tracing::warn!(payout = id, method = other, "payout method not implemented"),
        }
    }

    async fn send_lightning(&self, row: PayoutRow) {
        let Some(lnd) = &self.lightning else {
            return;
        };
        let net_msat = (row.amount - row.fee).max(0) as u64;
        let resolved = match resolve_destination(&self.http, &row.destination, net_msat).await {
            Ok(r) => r,
            Err(e) => {
                let _ = self.db.fail_unsent_payout(row.id, &e.to_string()).await;
                return;
            }
        };
        if self.db.mark_payout_sent(row.id, &resolved.payment_hash, None).await.is_err() {
            return;
        }
        match lnd.pay(&resolved.bolt11).await {
            Ok(_) => {
                let _ = self.db.complete_payout(row.id).await;
            }
            Err(e) if e.contains("Payment failed") => {
                let _ = self.db.fail_payout(row.id, &e).await;
            }
            Err(e) => tracing::error!(payout = row.id, error = %e, "lightning payout outcome unknown, left as sent"),
        }
    }

    async fn send_revolut(&self, row: PayoutRow) {
        let Some(revolut) = &self.revolut else {
            return;
        };
        let Some(account) = self.config.revolut_account(&row.currency) else {
            let _ = self.db.fail_unsent_payout(row.id, "no revolut account for currency").await;
            return;
        };
        let net = row.amount - row.fee;
        let created = revolut
            .create_payout_link(NewLink {
                payout_id: row.id,
                recipient_name: &row.destination,
                currency: &row.currency,
                amount_milli: net,
                account_id: account,
            })
            .await;
        match created {
            Ok(link) => self.record_link(row.id, link).await,
            Err(RevolutError::Api { message, .. }) => {
                let _ = self.db.fail_unsent_payout(row.id, &message).await;
            }
            Err(e) => tracing::warn!(payout = row.id, error = %e, "revolut link creation will be retried"),
        }
    }

    async fn record_link(&self, id: i64, link: PayoutLink) {
        if link.state.is_dead() {
            let _ = self.db.fail_unsent_payout(id, "revolut link was not created").await;
            return;
        }
        let url = link.url.as_deref();
        let _ = self.db.mark_payout_sent(id, &link.id, url).await;
        if link.state == LinkState::Processed {
            let _ = self.db.complete_payout(id).await;
        }
    }

    pub async fn sync_link(&self, row: &PayoutRow) {
        let (Some(revolut), Some(link_id)) = (&self.revolut, row.external_id.as_deref()) else {
            return;
        };
        let Ok(link) = revolut.get_payout_link(link_id).await else {
            return;
        };
        match link.state {
            LinkState::Processed => {
                let _ = self.db.complete_payout(row.id).await;
            }
            s if s.is_dead() => {
                let reason = link.cancellation_reason.unwrap_or_else(|| format!("link {s:?}").to_lowercase());
                let _ = self.db.fail_payout(row.id, &reason).await;
            }
            _ => {}
        }
    }

    pub async fn handle_webhook(&self, link_id: &str) {
        if let Ok(Some(row)) = self.db.get_payout_by_external(REVOLUT_LINK, link_id).await {
            if row.status == SENT {
                self.sync_link(&row).await;
            }
        }
    }

    pub async fn tick(&self) {
        for method in [LIGHTNING, REVOLUT_LINK] {
            for row in self.db.held_payouts(method).await.unwrap_or_default() {
                self.process(row.id).await;
            }
        }
        for row in self.db.stale_sent_payouts(REVOLUT_LINK, 5).await.unwrap_or_default() {
            self.sync_link(&row).await;
        }
    }

    pub fn spawn_worker(self) {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                self.tick().await;
            }
        });
    }

    pub async fn withdrawable(&self, user_id: i64, currency: &str) -> Result<i64, sqlx::Error> {
        self.db.withdrawable(user_id, currency, self.config.hold_days).await
    }

}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::payouts::{COMPLETED, REQUESTED};
    use crate::payouts::config::RevolutBusinessConfig;
    use crate::payouts::lightning::Paid;
    use crate::payouts::test_invoice::invoice;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};


    #[derive(Default)]
    struct MockLnd {
        paid: Mutex<Vec<String>>,
        fail: bool,
    }

    #[async_trait]
    impl InvoicePayer for MockLnd {
        async fn pay(&self, bolt11: &str) -> Result<Paid, String> {
            self.paid.lock().unwrap().push(bolt11.to_string());
            if self.fail {
                Err("Payment failed: FAILURE_REASON_NO_ROUTE".into())
            } else {
                Ok(Paid { payment_hash: "h".into(), fee_msat: 10 })
            }
        }
    }

    fn config(revolut: Option<&str>) -> PayoutsConfig {
        PayoutsConfig {
            enabled: true,
            min_sats: 1,
            approval_threshold_sats: 1_000_000,
            hold_days: 0,
            revolut: revolut.map(|base| RevolutBusinessConfig {
                base_url: base.into(),
                api_key: "k".into(),
                webhook_secret: "s".into(),
                account_ids: [("eur".to_string(), "acc".to_string())].into(),
            }),
            ..PayoutsConfig::default()
        }
    }

    async fn earn(db: &Database, user: i64, currency: &str, amount: i64) {
        db.credit_currency(user, currency, amount, "provider_earning", "e", 0).await.unwrap();
    }

    async fn already_approved_once(db: &Database, user: i64, currency: &str) {
        db.execute_raw(&format!(
            "INSERT INTO payouts (user_id, currency, amount, method, destination, status, external_id) \
             VALUES ({user}, '{currency}', 0, 'lightning', 'x', 'completed', 'seed')"
        ))
        .await
        .unwrap();
    }

    fn request<'a>(user: i64, currency: &'a str, amount: i64, method: &'a str, dest: &'a str) -> PayoutRequest<'a> {
        PayoutRequest { user_id: user, currency, amount, method, destination: dest }
    }

    #[tokio::test]
    async fn lightning_payout_pays_the_exact_net_amount_and_completes() {
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "sats", 600_000_000).await;
        already_approved_once(&db, u.id, "sats").await;
        let lnd = Arc::new(MockLnd::default());
        let service = PayoutService::new(db.clone(), config(None), Some(lnd.clone()), None);
        let bolt11 = invoice(Some(250_000_000), 3);
        let row = service.request(request(u.id, "sats", 250_000_000, LIGHTNING, &bolt11)).await.unwrap();
        assert_eq!(row.status, COMPLETED);
        assert_eq!(lnd.paid.lock().unwrap().len(), 1);
        assert_eq!(db.get_currency_balance(u.id, "sats").await.unwrap(), 350_000_000);
    }

    #[tokio::test]
    async fn an_operator_settles_a_held_lightning_payout_once_the_gateway_sent_it() {
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "sats", 600_000_000).await;
        already_approved_once(&db, u.id, "sats").await;
        let service = PayoutService::new(db.clone(), config(None), None, None);
        let bolt11 = invoice(Some(250_000_000), 5);
        let row = service.request(request(u.id, "sats", 250_000_000, LIGHTNING, &bolt11)).await.unwrap();
        assert_eq!(row.status, HELD);
        let done = service.settle(row.id, "gateway-withdrawal-9").await.unwrap();
        assert_eq!((done.status.as_str(), done.external_id.as_deref()), (COMPLETED, Some("gateway-withdrawal-9")));
        assert_eq!(db.get_currency_balance(u.id, "sats").await.unwrap(), 350_000_000);
        assert_eq!(service.settle(row.id, "again").await.unwrap_err(), PayoutError::WrongState);
    }

    #[tokio::test]
    async fn lightning_failure_returns_the_held_amount() {
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "sats", 600_000_000).await;
        already_approved_once(&db, u.id, "sats").await;
        let lnd = Arc::new(MockLnd { fail: true, ..Default::default() });
        let service = PayoutService::new(db.clone(), config(None), Some(lnd), None);
        let id = db
            .request_payout(
                NewPayout {
                    user_id: u.id,
                    currency: "sats",
                    amount: 250_000_000,
                    method: LIGHTNING,
                    destination: &invoice(Some(250_000_000), 4),
                    external_id: None,
                },
                &config(None).rules("sats"),
            )
            .await
            .unwrap()
            .id;
        service.process(id).await;
        let row = db.get_payout(id).await.unwrap().unwrap();
        assert_eq!(row.status, "failed");
        assert_eq!(db.get_currency_balance(u.id, "sats").await.unwrap(), 600_000_000);
    }

    #[tokio::test]
    async fn revolut_link_is_created_once_then_completed_by_webhook_or_poll() {
        let server = MockServer::start().await;
        let created = r#"{"id":"L9","state":"active","url":"https://business.revolut.com/p/abc","expiry_date":"2030-01-01T00:00:00Z"}"#;
        Mock::given(method("POST"))
            .and(path("/payout-links"))
            .respond_with(ResponseTemplate::new(201).set_body_raw(created, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/payout-links/L9"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"id":"L9","state":"processed","transaction_id":"T"}"#, "application/json"),
            )
            .mount(&server)
            .await;
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "eur", 50_000_000).await;
        already_approved_once(&db, u.id, "eur").await;
        let cfg = PayoutsConfig { min_fiat: 1, approval_threshold_fiat: 10_000, ..config(Some(&server.uri())) };
        let service = PayoutService::new(db.clone(), cfg, None, Some(RevolutBusiness::new(config(Some(&server.uri())).revolut.as_ref().unwrap())));
        let row = service.request(request(u.id, "eur", 12_000_000, REVOLUT_LINK, "Jane Doe")).await.unwrap();
        assert_eq!(row.status, "sent");
        assert_eq!(row.external_id.as_deref(), Some("L9"));
        assert_eq!(row.destination, "https://business.revolut.com/p/abc");
        assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 38_000_000);
        service.process(row.id).await;
        service.handle_webhook("L9").await;
        service.handle_webhook("L9").await;
        service.tick().await;
        let done = db.get_payout(row.id).await.unwrap().unwrap();
        assert_eq!(done.status, COMPLETED);
        assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 38_000_000);
    }

    #[tokio::test]
    async fn expired_revolut_link_returns_the_money_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/payout-links"))
            .respond_with(
                ResponseTemplate::new(201).set_body_raw(r#"{"id":"L1","state":"active","url":"https://x"}"#, "application/json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/payout-links/L1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"id":"L1","state":"expired"}"#, "application/json"))
            .mount(&server)
            .await;
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "eur", 50_000_000).await;
        already_approved_once(&db, u.id, "eur").await;
        let revolut_cfg = config(Some(&server.uri())).revolut.unwrap();
        let cfg = PayoutsConfig { min_fiat: 1, approval_threshold_fiat: 10_000, ..config(Some(&server.uri())) };
        let service = PayoutService::new(db.clone(), cfg, None, Some(RevolutBusiness::new(&revolut_cfg)));
        let row = service.request(request(u.id, "eur", 12_000_000, REVOLUT_LINK, "Jane")).await.unwrap();
        service.handle_webhook("L1").await;
        service.handle_webhook("L1").await;
        assert_eq!(db.get_payout(row.id).await.unwrap().unwrap().status, "failed");
        assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 50_000_000);
    }

    #[tokio::test]
    async fn first_payout_waits_for_admin_approval_then_sends() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/payout-links"))
            .respond_with(
                ResponseTemplate::new(201).set_body_raw(r#"{"id":"L2","state":"active","url":"https://x"}"#, "application/json"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        earn(&db, u.id, "usd", 50_000_000).await;
        let mut cfg = config(Some(&server.uri()));
        cfg.min_fiat = 1;
        cfg.revolut.as_mut().unwrap().account_ids.insert("usd".into(), "acc2".into());
        let revolut = RevolutBusiness::new(cfg.revolut.as_ref().unwrap());
        let service = PayoutService::new(db.clone(), cfg, None, Some(revolut));
        let row = service.request(request(u.id, "usd", 12_000_000, REVOLUT_LINK, "Jane")).await.unwrap();
        assert_eq!(row.status, REQUESTED);
        assert_eq!(db.get_currency_balance(u.id, "usd").await.unwrap(), 50_000_000);
        let sent = service.approve(row.id).await.unwrap();
        assert_eq!(sent.status, "sent");
        assert!(service.approve(row.id).await.is_err());
    }

    #[tokio::test]
    async fn disabled_or_unconfigured_methods_are_refused() {
        let db = Arc::new(Database::test().await);
        let u = db.add_test_user("u", false).await;
        let off = PayoutService::new(db.clone(), PayoutsConfig::default(), None, None);
        assert!(matches!(off.request(request(u.id, "eur", 1, REVOLUT_LINK, "x")).await, Err(RequestError::Disabled)));
        let on = PayoutService::new(db, config(None), None, None);
        assert!(matches!(
            on.request(request(u.id, "eur", 1, REVOLUT_LINK, "x")).await,
            Err(RequestError::MethodUnavailable(_))
        ));
    }
}
