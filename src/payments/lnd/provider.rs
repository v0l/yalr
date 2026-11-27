use super::LndService;
use crate::db::gateway_payments::{GatewayPayment, PENDING};
use crate::db::SATS;
use crate::payments::gateway::{check_sats, SATS_MAX, SATS_MIN};
use crate::payments::provider::{PaymentError, PaymentProvider, ProviderKind, TopupRequest, CARDS_NEED_GATEWAY};
use async_trait::async_trait;

#[async_trait]
impl PaymentProvider for LndService {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Lnd
    }

    fn mode(&self) -> &'static str {
        match self.settings.network.as_str() {
            "testnet" => "testnet",
            "signet" => "signet",
            "regtest" => "regtest",
            _ => "mainnet",
        }
    }

    fn currencies(&self) -> Vec<String> {
        vec![SATS.to_string()]
    }

    fn amount_range(&self) -> (i64, i64) {
        (SATS_MIN, SATS_MAX)
    }

    fn check_amount(&self, currency: &str, amount: f64) -> Result<i64, PaymentError> {
        if currency != SATS {
            return Err(PaymentError::Invalid(CARDS_NEED_GATEWAY.to_string()));
        }
        check_sats(amount)
    }

    async fn create_topup(&self, request: TopupRequest<'_>) -> Result<GatewayPayment, PaymentError> {
        self.create_invoice(request).await
    }

    async fn refresh(&self, payment: &GatewayPayment) -> Result<GatewayPayment, PaymentError> {
        if payment.status != PENDING {
            return Ok(payment.clone());
        }
        let _ = self.refresh_one(payment).await?;
        Ok(self.db.gateway_payment(&payment.public_id).await?.unwrap_or_else(|| payment.clone()))
    }

    async fn refund(&self, _user_id: i64, _amount_msat: i64, invoice: &str) -> Result<String, String> {
        self.pay(invoice).await.map(|paid| paid.payment_hash)
    }
}
