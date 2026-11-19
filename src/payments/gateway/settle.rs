use super::{local_currency, remote_amount, GatewayError, GatewayService, RemotePaymentView};
use payment_gateway_client::{verify_webhook, SIGNATURE_HEADER};
use crate::db::gateway_payments::{GatewayPayment, CANCELLED, EXPIRED, FAILED, KIND_AUTO_RELOAD};
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    data: EventData,
}

#[derive(Deserialize)]
struct EventData {
    payment: Option<RemotePaymentView>,
}

impl GatewayService {
    pub(crate) fn matches(&self, local: &GatewayPayment, remote: &RemotePaymentView) -> bool {
        local_currency(&remote.currency) == local.currency && remote.amount as i64 == remote_amount(&local.currency, local.amount_milli)
    }

    pub async fn reconcile(&self, remote_id: &str) -> Result<Option<GatewayPayment>, GatewayError> {
        if self.db.gateway_payment_by_remote(remote_id).await?.is_none() {
            return Ok(None);
        }
        let remote = self.client.get_payment(remote_id).await?;
        self.apply(&remote).await
    }

    pub(crate) async fn apply(&self, remote: &RemotePaymentView) -> Result<Option<GatewayPayment>, GatewayError> {
        let Some(local) = self.db.gateway_payment_by_remote(&remote.id).await? else {
            return Ok(None);
        };
        let outcome = match remote.status.as_str() {
            "paid" => {
                if self.matches(&local, remote) {
                    self.db.complete_gateway_payment(&remote.id).await?
                } else {
                    tracing::error!(payment = %local.public_id, "Gateway payment amount or currency differs from ours, not crediting");
                    self.db.close_gateway_payment(&remote.id, FAILED, Some("amount mismatch")).await?
                }
            }
            "expired" => self.db.close_gateway_payment(&remote.id, EXPIRED, None).await?,
            "cancelled" => self.db.close_gateway_payment(&remote.id, CANCELLED, None).await?,
            "failed" => self.db.close_gateway_payment(&remote.id, FAILED, Some("payment failed")).await?,
            _ => None,
        };
        if let Some(closed) = outcome.as_ref().filter(|o| o.kind == KIND_AUTO_RELOAD) {
            self.db.note_auto_reload_outcome(closed.user_id, &closed.currency, closed.status == crate::db::gateway_payments::COMPLETED).await?;
        }
        Ok(outcome)
    }

    pub async fn handle_webhook(&self, headers: &HeaderMap, body: &[u8]) -> StatusCode {
        let header = headers.get(SIGNATURE_HEADER).and_then(|v| v.to_str().ok());
        if let Err(e) = verify_webhook(body, header, &self.webhook_secret, 300) {
            tracing::warn!(error = %e, "Rejected gateway webhook");
            return if matches!(e, payment_gateway_client::WebhookError::InvalidBody) { StatusCode::BAD_REQUEST } else { StatusCode::UNAUTHORIZED };
        }
        let Ok(event) = serde_json::from_slice::<Event>(body) else {
            return StatusCode::BAD_REQUEST;
        };
        let Some(payment) = event.data.payment.filter(|_| event.kind.starts_with("payment.") || event.kind == "attempt.failed") else {
            return StatusCode::OK;
        };
        match self.apply(&payment).await {
            Ok(_) => StatusCode::OK,
            Err(e) => {
                tracing::warn!(payment = %payment.id, error = %e, "Gateway webhook could not be applied");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
}
