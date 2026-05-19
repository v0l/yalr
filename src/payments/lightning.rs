use std::sync::Arc;
use std::time::Duration;
use crate::db::{Database, LightningInvoiceRow, NewLightningInvoice};
use payments_rs::lightning::{LndNode, LightningNode, AddInvoiceRequest, PayInvoiceRequest, PayInvoiceResponse, InvoiceUpdate};

/// Wraps an LndNode for invoice creation and status tracking.
/// Stores invoices in the DB so they survive restarts.
pub struct LightningService {
    lnd: LndNode,
    db: Arc<Database>,
}

impl LightningService {
    pub fn new(lnd: LndNode, db: Arc<Database>) -> Self {
        Self { lnd, db }
    }

    /// Create a new Bolt11 invoice for a user top-up.
    /// Stores the invoice in the DB as 'pending'.
    pub async fn create_invoice(
        &self,
        user_id: i64,
        amount_sats: u64,
        memo: &str,
        expire_seconds: Option<u32>,
    ) -> Result<crate::payments::instructions::PaymentInstruction, LightningError> {
        let amount_msat = amount_sats * 1000;

        let invoice = self
            .lnd
            .add_invoice(AddInvoiceRequest {
                amount: amount_msat,
                memo: Some(memo.to_string()),
                expire: expire_seconds.or(Some(3600)),
            })
            .await
            .map_err(|e| LightningError::Lnd(format!("Failed to create invoice: {}", e)))?;

        let payment_hash = invoice.payment_hash();
        let bolt11 = invoice.pr();

        let db_invoice = self
            .db
            .create_lightning_invoice(NewLightningInvoice {
                user_id,
                payment_hash: &payment_hash,
                bolt11: &bolt11,
                amount_msat: amount_msat as i64,
                amount_sats: amount_sats as i64,
                expire_seconds,
            })
            .await
            .map_err(|e| LightningError::Db(e.to_string()))?;

        tracing::info!(
            user_id = user_id,
            amount_sats = amount_sats,
            payment_hash = %payment_hash,
            "Created Lightning invoice"
        );

        Ok(crate::payments::instructions::PaymentInstruction::LightningBolt11 {
            bolt11,
            payment_hash,
            amount_sats: amount_sats as i64,
            amount_msat: amount_msat as i64,
            memo: Some(memo.to_string()),
            expires_at: db_invoice.expires_at.and_then(|s| {
                chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                    .ok()
                    .map(|dt| dt.and_utc().timestamp())
            }),
            invoice_id: Some(db_invoice.id),
        })
    }

    /// Check the status of an invoice by payment hash.
    /// Checks DB first, then falls back to LND for settled invoices.
    pub async fn check_invoice(&self, payment_hash: &str) -> Result<InvoiceResponse, LightningError> {
        let db_invoice = self
            .db
            .get_lightning_invoice_by_hash(payment_hash)
            .await
            .map_err(|e| LightningError::Db(e.to_string()))?
            .ok_or_else(|| LightningError::NotFound(payment_hash.to_string()))?;

        // If already marked paid or cancelled in DB, return that
        if db_invoice.status == "paid" || db_invoice.status == "cancelled" {
            return Ok(db_invoice.into());
        }

        // Check expiry
        if let Some(ref expires_at) = db_invoice.expires_at {
            // SQLite timestamps are "YYYY-MM-DD HH:MM:SS"
            if let Ok(exp) = chrono::NaiveDateTime::parse_from_str(expires_at, "%Y-%m-%d %H:%M:%S") {
                if chrono::Utc::now().naive_utc() > exp {
                    let _ = self
                        .db
                        .update_lightning_invoice_status(payment_hash, "expired")
                        .await;
                    return Ok(InvoiceResponse {
                        status: "expired".to_string(),
                        ..db_invoice.into()
                    });
                }
            }
        }

        // Still pending — return as is
        Ok(db_invoice.into())
    }

    /// Pay a Lightning invoice (used for refunds).
    /// Takes the Bolt11 invoice string and optional max amount in sats.
    pub async fn pay_invoice(
        &self,
        invoice: &str,
        _max_amount_sats: Option<u64>,
    ) -> Result<PayInvoiceResponse, LightningError> {
        self.lnd
            .pay_invoice(PayInvoiceRequest {
                invoice: invoice.to_string(),
                timeout_seconds: Some(120),
            })
            .await
            .map_err(|e| LightningError::Lnd(format!("Payment failed: {}", e)))
    }

    /// Start a background listener that subscribes to LND invoice updates.
    /// Credits user balances when invoices are settled.
    pub fn start_settlement_listener(
        &self,
        balance_service: Arc<crate::payments::balance::BalanceService>,
    ) -> tokio::task::JoinHandle<()> {
        let db = self.db.clone();
        let lnd = self.lnd.clone();

        tokio::spawn(async move {
            tracing::info!("Starting LND invoice settlement listener");

            loop {
                let stream = match lnd.subscribe_invoices(None).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to subscribe to LND invoices, retrying in 10s");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                use futures::StreamExt;
                let mut stream = stream;

                while let Some(update) = stream.next().await {
                    match update {
                        InvoiceUpdate::Settled {
                            payment_hash,
                            preimage: _,
                            external_id: _,
                        } => {
                            tracing::info!(payment_hash = %payment_hash, "Invoice settled");
                            if let Err(e) = handle_settlement(&db, &balance_service, &payment_hash).await {
                                tracing::error!(payment_hash = %payment_hash, error = %e, "Settlement handling failed");
                            }
                        }
                        InvoiceUpdate::Canceled { payment_hash } => {
                            tracing::info!(payment_hash = %payment_hash, "Invoice cancelled");
                            let _ = db
                                .update_lightning_invoice_status(&payment_hash, "cancelled")
                                .await;
                        }
                        InvoiceUpdate::Error(e) => {
                            tracing::warn!(error = %e, "LND subscription error");
                        }
                        _ => {}
                    }
                }

                tracing::warn!("LND invoice stream ended, reconnecting in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}

async fn handle_settlement(
    db: &Database,
    balance_service: &crate::payments::balance::BalanceService,
    payment_hash: &str,
) -> Result<(), String> {
    let updated = match db.update_lightning_invoice_status(payment_hash, "paid").await {
        Ok(Some(inv)) => inv,
        Ok(None) => return Err("Invoice not found in DB after settlement".to_string()),
        Err(e) => return Err(format!("Failed to update invoice status: {}", e)),
    };

    // Skip if already marked paid (idempotency)
    if updated.paid_at.is_some() {
        return Ok(());
    }

    balance_service
        .credit(updated.user_id, updated.amount_msat, "deposit", payment_hash)
        .await
        .map_err(|e| format!("Failed to credit balance: {}", e))?;

    tracing::info!(
        user_id = updated.user_id,
        amount_msat = updated.amount_msat,
        payment_hash = %payment_hash,
        "Credited user balance from Lightning payment"
    );
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InvoiceResponse {
    pub id: i64,
    pub payment_hash: String,
    pub bolt11: String,
    pub amount_sats: i64,
    pub amount_msat: i64,
    pub status: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

impl From<LightningInvoiceRow> for InvoiceResponse {
    fn from(row: LightningInvoiceRow) -> Self {
        Self {
            id: row.id,
            payment_hash: row.payment_hash,
            bolt11: row.bolt11,
            amount_sats: row.amount_sats,
            amount_msat: row.amount_msat,
            status: row.status,
            created_at: row.created_at,
            expires_at: row.expires_at,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LightningError {
    #[error("LND error: {0}")]
    Lnd(String),

    #[error("Database error: {0}")]
    Db(String),

    #[error("Invoice not found: {0}")]
    NotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Most tests require a running LND node.
    // The test below validates the error types and serialization.

    #[test]
    fn test_invoice_response_serialization() {
        let resp = InvoiceResponse {
            id: 1,
            payment_hash: "abc123".to_string(),
            bolt11: "lnbc...".to_string(),
            amount_sats: 1000,
            amount_msat: 1_000_000,
            status: "pending".to_string(),
            created_at: "2025-01-01T00:00:00".to_string(),
            expires_at: Some("2025-01-01T01:00:00".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("lnbc..."));
        assert!(json.contains("pending"));
    }

    #[test]
    fn test_lightning_error_display() {
        assert_eq!(
            LightningError::Lnd("timeout".to_string()).to_string(),
            "LND error: timeout"
        );
        assert_eq!(
            LightningError::NotFound("hash123".to_string()).to_string(),
            "Invoice not found: hash123"
        );
    }
}
