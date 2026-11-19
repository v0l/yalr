use super::currency::{add, record};
use super::{Database, Timestamp};

pub const PENDING: &str = "pending";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";
pub const EXPIRED: &str = "expired";
pub const KIND_TOPUP: &str = "topup";
pub const KIND_AUTO_RELOAD: &str = "auto_reload";
pub const KIND_ANONYMOUS: &str = "anonymous";

const COLUMNS: &str = "id, public_id, gateway_payment_id, user_id, kind, purpose, currency, amount_milli, status, \
    bolt11, payment_hash, pay_url, failure_reason, created_at, updated_at, expires_at, completed_at";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct GatewayPayment {
    pub id: i64,
    pub public_id: String,
    pub gateway_payment_id: Option<String>,
    pub user_id: i64,
    pub kind: String,
    pub purpose: Option<String>,
    pub currency: String,
    pub amount_milli: i64,
    pub status: String,
    pub bolt11: Option<String>,
    pub payment_hash: Option<String>,
    pub pay_url: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub expires_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct GatewayPaymentWithUser {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub payment: GatewayPayment,
    pub username: String,
}

pub struct NewGatewayPayment<'a> {
    pub public_id: &'a str,
    pub user_id: i64,
    pub kind: &'a str,
    pub purpose: Option<&'a str>,
    pub currency: &'a str,
    pub amount_milli: i64,
}

pub struct RemotePayment<'a> {
    pub gateway_payment_id: &'a str,
    pub pay_url: Option<&'a str>,
    pub bolt11: Option<&'a str>,
    pub payment_hash: Option<&'a str>,
    pub expires_at: Option<Timestamp>,
}

impl Database {
    pub async fn create_gateway_payment(&self, new: NewGatewayPayment<'_>) -> Result<GatewayPayment, sqlx::Error> {
        sqlx::query(
            "INSERT INTO gateway_payments (public_id, user_id, kind, purpose, currency, amount_milli) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(new.public_id)
        .bind(new.user_id)
        .bind(new.kind)
        .bind(new.purpose)
        .bind(new.currency)
        .bind(new.amount_milli)
        .execute(&self.pool)
        .await?;
        Ok(self.gateway_payment(new.public_id).await?.expect("payment just inserted"))
    }

    pub async fn attach_remote_payment(&self, id: i64, remote: RemotePayment<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE gateway_payments SET gateway_payment_id = ?, pay_url = COALESCE(?, pay_url), \
             bolt11 = COALESCE(?, bolt11), payment_hash = COALESCE(?, payment_hash), expires_at = COALESCE(?, expires_at) \
             WHERE id = ?",
        )
        .bind(remote.gateway_payment_id)
        .bind(remote.pay_url)
        .bind(remote.bolt11)
        .bind(remote.payment_hash)
        .bind(remote.expires_at)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn gateway_payment(&self, public_id: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        self.gateway_payment_where("public_id", public_id).await
    }

    pub async fn gateway_payment_by_remote(&self, remote_id: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        self.gateway_payment_where("gateway_payment_id", remote_id).await
    }

    pub async fn gateway_payment_by_hash(&self, payment_hash: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        self.gateway_payment_where("payment_hash", payment_hash).await
    }

    pub async fn gateway_payment_by_bolt11(&self, bolt11: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        self.gateway_payment_where("bolt11", bolt11).await
    }

    async fn gateway_payment_where(&self, column: &str, value: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM gateway_payments WHERE {column} = ? LIMIT 1"))
            .bind(value)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_user_gateway_payments(&self, user_id: i64, limit: i64) -> Result<Vec<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM gateway_payments WHERE user_id = ? ORDER BY id DESC LIMIT ?"))
            .bind(user_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn list_gateway_payments(&self, user_id: Option<i64>, limit: i64) -> Result<Vec<GatewayPayment>, sqlx::Error> {
        let mut query = sqlx::QueryBuilder::new(format!("SELECT {COLUMNS} FROM gateway_payments"));
        if let Some(uid) = user_id {
            query.push(" WHERE user_id = ").push_bind(uid);
        }
        query.push(" ORDER BY created_at DESC, id DESC LIMIT ").push_bind(limit);
        query.build_query_as().fetch_all(&self.pool).await
    }

    pub async fn list_all_gateway_payments(&self, limit: i64) -> Result<Vec<GatewayPaymentWithUser>, sqlx::Error> {
        sqlx::query_as(
            "SELECT p.id, p.public_id, p.gateway_payment_id, p.user_id, p.kind, p.purpose, p.currency, p.amount_milli, \
             p.status, p.bolt11, p.payment_hash, p.pay_url, p.failure_reason, p.created_at, p.updated_at, p.expires_at, \
             p.completed_at, \
             COALESCE(NULLIF(u.display_name, ''), u.username, u.external_id, CONCAT('user-', p.user_id)) AS username \
             FROM gateway_payments p LEFT JOIN users u ON u.id = p.user_id ORDER BY p.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn stale_pending_gateway_payments(&self, older_than_secs: i64, limit: i64) -> Result<Vec<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM gateway_payments WHERE status = 'pending' AND gateway_payment_id IS NOT NULL \
             AND created_at < UTC_TIMESTAMP(3) - INTERVAL ? SECOND ORDER BY id LIMIT ?"
        ))
        .bind(older_than_secs)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn complete_gateway_payment(&self, remote_id: &str) -> Result<Option<GatewayPayment>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let payment: Option<GatewayPayment> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM gateway_payments WHERE gateway_payment_id = ? FOR UPDATE"
        ))
        .bind(remote_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(mut payment) = payment.filter(|p| p.status == PENDING || p.status == EXPIRED) else {
            tx.rollback().await?;
            return Ok(None);
        };
        sqlx::query(
            "UPDATE gateway_payments SET status = 'completed', completed_at = UTC_TIMESTAMP(3), failure_reason = NULL WHERE id = ?",
        )
        .bind(payment.id)
        .execute(&mut *tx)
        .await?;
        add(&mut tx, payment.user_id, &payment.currency, payment.amount_milli, payment.amount_milli).await?;
        let reference = format!("gw-{}", payment.public_id);
        record(&mut tx, payment.user_id, &payment.currency, payment.amount_milli, "deposit", &reference, Some(&payment.kind)).await?;
        tx.commit().await?;
        payment.status = COMPLETED.to_string();
        Ok(Some(payment))
    }

    pub async fn close_gateway_payment(&self, remote_id: &str, status: &str, reason: Option<&str>) -> Result<Option<GatewayPayment>, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE gateway_payments SET status = ?, failure_reason = ? WHERE gateway_payment_id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(reason)
        .bind(remote_id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.gateway_payment_where("gateway_payment_id", remote_id).await
    }

    pub async fn fail_unsent_gateway_payment(&self, id: i64, reason: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE gateway_payments SET status = 'failed', failure_reason = ? WHERE id = ? AND status = 'pending'")
            .bind(reason)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn expire_unsent_gateway_payments(&self, older_than_secs: i64) -> Result<u64, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE gateway_payments SET status = 'failed', failure_reason = 'payment was never sent to the gateway' \
             WHERE status = 'pending' AND gateway_payment_id IS NULL AND created_at < UTC_TIMESTAMP(3) - INTERVAL ? SECOND",
        )
        .bind(older_than_secs)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    pub async fn recent_completed_gateway_payments(&self, user_id: i64, currency: &str, limit: i64) -> Result<Vec<GatewayPayment>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM gateway_payments WHERE user_id = ? AND currency = ? AND status = 'completed' \
             AND gateway_payment_id IS NOT NULL ORDER BY id DESC LIMIT ?"
        ))
        .bind(user_id)
        .bind(currency)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn payment(db: &Database, user: i64, remote: &str, currency: &str, amount_milli: i64) -> GatewayPayment {
        let created = db
            .create_gateway_payment(NewGatewayPayment {
                public_id: &super::super::random_hex(),
                user_id: user,
                kind: KIND_TOPUP,
                purpose: None,
                currency,
                amount_milli,
            })
            .await
            .unwrap();
        db.attach_remote_payment(created.id, RemotePayment { gateway_payment_id: remote, pay_url: None, bolt11: None, payment_hash: None, expires_at: None })
            .await
            .unwrap();
        created
    }

    #[tokio::test]
    async fn completing_credits_millicents_once_however_often_it_is_confirmed() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let created = payment(&db, user.id, "gw_1", "eur", 25_000_000).await;

        assert_eq!(db.complete_gateway_payment("gw_1").await.unwrap().map(|p| p.status), Some("completed".to_string()));
        assert!(db.complete_gateway_payment("gw_1").await.unwrap().is_none());
        assert!(db.complete_gateway_payment("gw_1").await.unwrap().is_none());

        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 25_000_000);
        let rows = db.list_transactions_for_reference(&format!("gw-{}", created.public_id)).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].currency.as_str(), rows[0].amount_milli, rows[0].transaction_type.as_str()), ("eur", 25_000_000, "deposit"));
    }

    #[tokio::test]
    async fn sats_payments_credit_the_msat_balance() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        payment(&db, user.id, "gw_s", "sats", 5_000_000).await;
        db.complete_gateway_payment("gw_s").await.unwrap().unwrap();
        assert_eq!(db.get_currency_balance(user.id, "sats").await.unwrap(), 5_000_000);
    }

    #[tokio::test]
    async fn concurrent_confirmations_credit_once() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        payment(&db, user.id, "gw_2", "eur", 25_000_000).await;
        let attempts: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                tokio::spawn(async move { db.complete_gateway_payment("gw_2").await.unwrap().is_some() })
            })
            .collect();
        let mut credited = 0;
        for attempt in attempts {
            credited += attempt.await.unwrap() as i32;
        }
        assert_eq!(credited, 1);
        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 25_000_000);
    }

    #[tokio::test]
    async fn failed_and_cancelled_payments_credit_nothing_even_if_paid_later() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        payment(&db, user.id, "gw_f", "eur", 25_000_000).await;
        payment(&db, user.id, "gw_c", "eur", 25_000_000).await;
        assert!(db.close_gateway_payment("gw_f", FAILED, Some("declined")).await.unwrap().is_some());
        assert!(db.close_gateway_payment("gw_c", CANCELLED, None).await.unwrap().is_some());
        assert!(db.close_gateway_payment("gw_c", FAILED, None).await.unwrap().is_none());
        assert!(db.complete_gateway_payment("gw_f").await.unwrap().is_none());
        assert!(db.complete_gateway_payment("gw_c").await.unwrap().is_none());
        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_late_payment_on_an_expired_row_is_still_credited() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let created = payment(&db, user.id, "gw_e", "eur", 1_000_000).await;
        db.execute_raw(&format!("UPDATE gateway_payments SET status = 'expired' WHERE id = {}", created.id)).await.unwrap();
        assert!(db.complete_gateway_payment("gw_e").await.unwrap().is_some());
        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 1_000_000);
    }

    #[tokio::test]
    async fn stale_pending_lists_only_old_pending_payments_with_a_gateway_id() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let old = payment(&db, user.id, "gw_old", "eur", 1_000).await;
        payment(&db, user.id, "gw_new", "eur", 1_000).await;
        db.execute_raw(&format!("UPDATE gateway_payments SET created_at = UTC_TIMESTAMP(3) - INTERVAL 1 HOUR WHERE id = {}", old.id))
            .await
            .unwrap();
        let stale = db.stale_pending_gateway_payments(600, 50).await.unwrap();
        assert_eq!(stale.iter().map(|p| p.gateway_payment_id.clone().unwrap()).collect::<Vec<_>>(), vec!["gw_old"]);
    }
}
