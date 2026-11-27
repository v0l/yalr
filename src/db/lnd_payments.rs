use super::gateway_payments::{GatewayPayment, NewGatewayPayment, PROVIDER_LND};
use super::Database;

const COLUMNS: &str = "id, public_id, gateway_payment_id, user_id, kind, purpose, currency, amount_milli, status, \
    bolt11, payment_hash, pay_url, failure_reason, created_at, updated_at, expires_at, completed_at";

impl Database {
    pub async fn create_lnd_payment(&self, new: NewGatewayPayment<'_>) -> Result<GatewayPayment, sqlx::Error> {
        self.insert_payment(new, PROVIDER_LND).await
    }

    pub async fn lnd_payment_by_hash(&self, payment_hash: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM gateway_payments WHERE gateway_payment_id = ? AND provider = 'lnd'"))
            .bind(payment_hash)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn pending_lnd_payments(&self, limit: i64) -> Result<Vec<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM gateway_payments WHERE provider = 'lnd' AND status = 'pending' \
             AND gateway_payment_id IS NOT NULL ORDER BY id LIMIT ?"
        ))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn payment_cursor(&self, name: &str) -> Result<u64, sqlx::Error> {
        let position: Option<u64> = sqlx::query_scalar("SELECT position FROM payment_cursors WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(position.unwrap_or(0))
    }

    pub async fn advance_payment_cursor(&self, name: &str, position: u64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO payment_cursors (name, position) VALUES (?, ?) \
             ON DUPLICATE KEY UPDATE position = GREATEST(position, VALUES(position))",
        )
        .bind(name)
        .bind(position)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::gateway_payments::{RemotePayment, KIND_TOPUP};
    use super::*;

    async fn lnd_row(db: &Database, user: i64, hash: &str) -> GatewayPayment {
        let row = db
            .create_lnd_payment(NewGatewayPayment {
                public_id: &super::super::random_hex(),
                user_id: user,
                kind: KIND_TOPUP,
                purpose: None,
                currency: "sats",
                amount_milli: 1_000_000,
            })
            .await
            .unwrap();
        db.attach_remote_payment(row.id, RemotePayment { gateway_payment_id: hash, pay_url: None, bolt11: Some("lnbc"), payment_hash: Some(hash), expires_at: None })
            .await
            .unwrap();
        row
    }

    #[tokio::test]
    async fn the_cursor_starts_at_zero_and_never_moves_back() {
        let db = Database::test().await;
        assert_eq!(db.payment_cursor("lnd").await.unwrap(), 0);
        db.advance_payment_cursor("lnd", 7).await.unwrap();
        db.advance_payment_cursor("lnd", 3).await.unwrap();
        assert_eq!(db.payment_cursor("lnd").await.unwrap(), 7);
        db.advance_payment_cursor("lnd", 9).await.unwrap();
        assert_eq!(db.payment_cursor("lnd").await.unwrap(), 9);
        assert_eq!(db.payment_cursor("other").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn lnd_rows_stay_out_of_the_gateway_reconciler_and_refund_lists() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let row = lnd_row(&db, user.id, "h1").await;
        db.execute_raw(&format!("UPDATE gateway_payments SET created_at = UTC_TIMESTAMP(3) - INTERVAL 1 HOUR WHERE id = {}", row.id))
            .await
            .unwrap();
        assert!(db.stale_pending_gateway_payments(60, 50).await.unwrap().is_empty());
        assert_eq!(db.pending_lnd_payments(50).await.unwrap().len(), 1);
        db.complete_gateway_payment("h1").await.unwrap().unwrap();
        assert!(db.pending_lnd_payments(50).await.unwrap().is_empty());
        assert!(db.recent_completed_gateway_payments(user.id, "sats", 20).await.unwrap().is_empty());
        assert_eq!(db.lnd_payment_by_hash("h1").await.unwrap().map(|p| p.status), Some("completed".to_string()));
    }
}
