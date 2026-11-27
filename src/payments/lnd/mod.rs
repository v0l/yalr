mod node;
mod provider;
mod watcher;
#[cfg(test)]
pub mod fake;
#[cfg(test)]
mod tests;

pub use node::{InvoiceEvent, InvoiceState, LightningNode, LndNode, NewInvoice, PaidInvoice};
pub use watcher::{spawn_follower, spawn_watcher};

use crate::config::LndSettings;
use crate::db::gateway_payments::{GatewayPayment, NewGatewayPayment, RemotePayment, EXPIRED, FAILED};
use crate::db::{random_hex, Database, SATS};
use crate::payments::provider::{PaymentError, TopupRequest};
use std::sync::Arc;

pub const SETTLE_CURSOR: &str = "lnd_settle_index";

pub struct LndService {
    pub(crate) node: Arc<dyn LightningNode>,
    pub(crate) db: Arc<Database>,
    pub settings: LndSettings,
}

impl LndService {
    pub fn new(node: Arc<dyn LightningNode>, db: Arc<Database>, settings: LndSettings) -> Self {
        Self { node, db, settings }
    }

    pub async fn connect(settings: LndSettings, db: Arc<Database>) -> Result<Self, String> {
        let node = LndNode::connect(&settings).await?;
        Ok(Self::new(Arc::new(node), db, settings))
    }

    pub async fn create_invoice(&self, request: TopupRequest<'_>) -> Result<GatewayPayment, PaymentError> {
        if request.currency != SATS {
            return Err(PaymentError::Invalid(crate::payments::provider::CARDS_NEED_GATEWAY.to_string()));
        }
        let public_id = request.public_id.map_or_else(random_hex, str::to_string);
        let order = self
            .db
            .create_lnd_payment(NewGatewayPayment {
                public_id: &public_id,
                user_id: request.user_id,
                kind: request.kind,
                purpose: request.purpose,
                currency: SATS,
                amount_milli: request.amount_milli,
            })
            .await?;
        let memo = format!("YALR balance top-up {public_id}");
        let invoice = match self.node.add_invoice(order.amount_milli as u64, &memo, self.settings.invoice_expiry_secs).await {
            Ok(invoice) => invoice,
            Err(e) => {
                self.db.fail_unsent_gateway_payment(order.id, "LND could not create the invoice").await?;
                return Err(PaymentError::Upstream(format!("LND could not create the invoice: {e}")));
            }
        };
        let expires_at = chrono::Utc::now() + chrono::Duration::seconds(self.settings.invoice_expiry_secs as i64);
        self.db
            .attach_remote_payment(
                order.id,
                RemotePayment {
                    gateway_payment_id: &invoice.payment_hash,
                    pay_url: None,
                    bolt11: Some(&invoice.bolt11),
                    payment_hash: Some(&invoice.payment_hash),
                    expires_at: Some(expires_at),
                },
            )
            .await?;
        Ok(self.db.gateway_payment(&public_id).await?.unwrap_or(order))
    }

    pub async fn apply(&self, event: &InvoiceEvent) -> Result<Option<GatewayPayment>, PaymentError> {
        let Some(local) = self.db.lnd_payment_by_hash(&event.payment_hash).await? else {
            return Ok(None);
        };
        let outcome = match event.state {
            InvoiceState::Settled if event.amt_paid_msat >= local.amount_milli => self.db.complete_gateway_payment(&event.payment_hash).await?,
            InvoiceState::Settled => {
                tracing::error!(payment = %local.public_id, paid = event.amt_paid_msat, "LND invoice settled for less than we asked, not crediting");
                self.db.close_gateway_payment(&event.payment_hash, FAILED, Some("amount mismatch")).await?
            }
            InvoiceState::Canceled => self.db.close_gateway_payment(&event.payment_hash, EXPIRED, None).await?,
            InvoiceState::Open if local.expires_at.is_some_and(|t| chrono::Utc::now() > t) => {
                self.db.close_gateway_payment(&event.payment_hash, EXPIRED, None).await?
            }
            InvoiceState::Open | InvoiceState::Accepted => None,
        };
        if let Some(done) = outcome.as_ref() {
            tracing::info!(payment = %done.public_id, status = %done.status, "LND invoice closed");
        }
        Ok(outcome)
    }

    pub async fn refresh_one(&self, payment: &GatewayPayment) -> Result<Option<GatewayPayment>, PaymentError> {
        let Some(hash) = payment.gateway_payment_id.as_deref() else {
            return Ok(None);
        };
        match self.node.lookup_invoice(hash).await.map_err(PaymentError::Upstream)? {
            Some(event) => self.apply(&event).await,
            None => Ok(None),
        }
    }

    pub async fn pay(&self, bolt11: &str) -> Result<PaidInvoice, String> {
        self.node.pay_invoice(bolt11).await
    }
}

#[async_trait::async_trait]
impl crate::payouts::lightning::InvoicePayer for LndService {
    async fn pay(&self, bolt11: &str) -> Result<crate::payouts::lightning::Paid, String> {
        let paid = LndService::pay(self, bolt11).await?;
        Ok(crate::payouts::lightning::Paid { payment_hash: paid.payment_hash, fee_msat: paid.fee_msat })
    }
}
