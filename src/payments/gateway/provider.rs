use super::GatewayService;
use crate::db::gateway_payments::{GatewayPayment, PENDING};
use crate::payments::provider::{PaymentError, PaymentProvider, ProviderKind, TopupRequest};
use async_trait::async_trait;
use axum::http::{HeaderMap, StatusCode};

const REFUND_DEPOSITS: i64 = 20;

#[async_trait]
impl PaymentProvider for GatewayService {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gateway
    }

    fn mode(&self) -> &'static str {
        GatewayService::mode(self)
    }

    fn currencies(&self) -> Vec<String> {
        self.settings.currencies.clone()
    }

    fn amount_range(&self) -> (i64, i64) {
        (self.settings.min_amount, self.settings.max_amount)
    }

    fn check_amount(&self, currency: &str, amount: f64) -> Result<i64, PaymentError> {
        GatewayService::check_amount(self, currency, amount)
    }

    async fn create_topup(&self, request: TopupRequest<'_>) -> Result<GatewayPayment, PaymentError> {
        GatewayService::create_topup(self, request).await
    }

    async fn refresh(&self, payment: &GatewayPayment) -> Result<GatewayPayment, PaymentError> {
        if payment.status != PENDING {
            return Ok(payment.clone());
        }
        let Some(remote) = payment.gateway_payment_id.as_deref() else {
            return Ok(payment.clone());
        };
        let _ = self.reconcile(remote).await;
        Ok(self.db.gateway_payment(&payment.public_id).await?.unwrap_or_else(|| payment.clone()))
    }

    async fn handle_webhook(&self, headers: &HeaderMap, body: &[u8]) -> StatusCode {
        GatewayService::handle_webhook(self, headers, body).await
    }

    async fn refund(&self, user_id: i64, amount_msat: i64, invoice: &str) -> Result<String, String> {
        let deposits = self
            .db
            .recent_completed_gateway_payments(user_id, crate::db::SATS, REFUND_DEPOSITS)
            .await
            .map_err(|e| e.to_string())?;
        let mut last = "no Lightning deposit is large enough to refund from".to_string();
        for deposit in deposits.into_iter().filter(|d| d.amount_milli >= amount_msat) {
            let Some(remote) = deposit.gateway_payment_id.as_deref() else { continue };
            let input = payment_gateway_client::RefundInput { amount: Some(amount_msat as u64), destination: Some(invoice.to_string()), attempt_id: None };
            match self.client.refund_payment(remote, &input).await {
                Ok(_) => return Ok(deposit.payment_hash.unwrap_or_default()),
                Err(e) => last = e.to_string(),
            }
        }
        Err(last)
    }

    async fn btc_usd(&self) -> Option<f64> {
        GatewayService::btc_usd(self).await
    }
}
