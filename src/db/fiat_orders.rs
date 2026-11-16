use super::currency::{add, record};
use super::{random_hex, Database, Timestamp};

pub const PENDING: &str = "pending";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";
pub const KIND_TOPUP: &str = "topup";
pub const KIND_AUTO_RELOAD: &str = "auto_reload";

const COLUMNS: &str = "id, public_id, user_id, provider, external_id, kind, currency, amount_milli, status, \
    save_card, failure_reason, created_at, updated_at, completed_at";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FiatOrder {
    pub id: i64,
    pub public_id: String,
    pub user_id: i64,
    pub provider: String,
    pub external_id: Option<String>,
    pub kind: String,
    pub currency: String,
    pub amount_milli: i64,
    pub status: String,
    pub save_card: bool,
    pub failure_reason: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FiatOrderWithUser {
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub order: FiatOrder,
    pub username: String,
}

pub struct NewFiatOrder<'a> {
    pub user_id: i64,
    pub kind: &'a str,
    pub currency: &'a str,
    pub amount_milli: i64,
    pub save_card: bool,
}

impl Database {
    pub async fn create_fiat_order(&self, new: NewFiatOrder<'_>) -> Result<FiatOrder, sqlx::Error> {
        let public_id = random_hex();
        sqlx::query(
            "INSERT INTO fiat_orders (public_id, user_id, kind, currency, amount_milli, save_card) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&public_id)
        .bind(new.user_id)
        .bind(new.kind)
        .bind(new.currency)
        .bind(new.amount_milli)
        .bind(new.save_card)
        .execute(&self.pool)
        .await?;
        self.fiat_order(&public_id).await.map(|o| o.expect("order just inserted"))
    }

    pub async fn set_fiat_order_external(&self, id: i64, external_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE fiat_orders SET external_id = ? WHERE id = ?")
            .bind(external_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn fiat_order(&self, public_id: &str) -> Result<Option<FiatOrder>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM fiat_orders WHERE public_id = ?"))
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn fiat_order_by_external(&self, external_id: &str) -> Result<Option<FiatOrder>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM fiat_orders WHERE provider = 'revolut' AND external_id = ?"))
            .bind(external_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_user_fiat_orders(&self, user_id: i64, limit: i64) -> Result<Vec<FiatOrder>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM fiat_orders WHERE user_id = ? AND external_id IS NOT NULL ORDER BY id DESC LIMIT ?"
        ))
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_all_fiat_orders(&self, limit: i64) -> Result<Vec<FiatOrderWithUser>, sqlx::Error> {
        sqlx::query_as(
            "SELECT o.id, o.public_id, o.user_id, o.provider, o.external_id, o.kind, o.currency, o.amount_milli, \
             o.status, o.save_card, o.failure_reason, o.created_at, o.updated_at, o.completed_at, \
             COALESCE(NULLIF(u.display_name, ''), u.username, u.external_id, CONCAT('user-', o.user_id)) AS username \
             FROM fiat_orders o LEFT JOIN users u ON u.id = o.user_id ORDER BY o.id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn stale_pending_fiat_orders(&self, older_than_secs: i64, limit: i64) -> Result<Vec<FiatOrder>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM fiat_orders WHERE status = 'pending' AND external_id IS NOT NULL \
             AND created_at < UTC_TIMESTAMP(3) - INTERVAL ? SECOND ORDER BY id LIMIT ?"
        ))
        .bind(older_than_secs)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn complete_fiat_order(&self, external_id: &str) -> Result<Option<FiatOrder>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let order: Option<FiatOrder> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM fiat_orders WHERE provider = 'revolut' AND external_id = ? FOR UPDATE"
        ))
        .bind(external_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(mut order) = order.filter(|o| o.status == PENDING) else {
            tx.rollback().await?;
            return Ok(None);
        };
        sqlx::query(
            "UPDATE fiat_orders SET status = 'completed', completed_at = UTC_TIMESTAMP(3), failure_reason = NULL WHERE id = ?",
        )
        .bind(order.id)
        .execute(&mut *tx)
        .await?;
        add(&mut tx, order.user_id, &order.currency, order.amount_milli, order.amount_milli).await?;
        let reference = format!("fiat-{}", order.public_id);
        record(&mut tx, order.user_id, &order.currency, order.amount_milli, "deposit", &reference, Some(&order.kind)).await?;
        tx.commit().await?;
        order.status = COMPLETED.to_string();
        Ok(Some(order))
    }

    pub async fn close_fiat_order(&self, external_id: &str, status: &str, reason: Option<&str>) -> Result<Option<FiatOrder>, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE fiat_orders SET status = ?, failure_reason = ? \
             WHERE provider = 'revolut' AND external_id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(reason)
        .bind(external_id)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(None);
        }
        self.fiat_order_by_external(external_id).await
    }

    pub async fn expire_unsent_fiat_orders(&self, older_than_secs: i64) -> Result<u64, sqlx::Error> {
        let done = sqlx::query(
            "UPDATE fiat_orders SET status = 'failed', failure_reason = 'order was never sent to the processor' \
             WHERE status = 'pending' AND external_id IS NULL AND created_at < UTC_TIMESTAMP(3) - INTERVAL ? SECOND",
        )
        .bind(older_than_secs)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    pub async fn fail_unsent_fiat_order(&self, id: i64, reason: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE fiat_orders SET status = 'failed', failure_reason = ? WHERE id = ? AND status = 'pending'")
            .bind(reason)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn order(db: &Database, user: i64, external: &str) -> FiatOrder {
        let order = db
            .create_fiat_order(NewFiatOrder { user_id: user, kind: KIND_TOPUP, currency: "eur", amount_milli: 25_000_000, save_card: false })
            .await
            .unwrap();
        db.set_fiat_order_external(order.id, external).await.unwrap();
        order
    }

    #[tokio::test]
    async fn completing_credits_millicents_once_however_often_it_is_confirmed() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let created = order(&db, user.id, "rev_1").await;

        let first = db.complete_fiat_order("rev_1").await.unwrap();
        assert_eq!(first.map(|o| o.status), Some("completed".to_string()));
        assert!(db.complete_fiat_order("rev_1").await.unwrap().is_none());
        assert!(db.complete_fiat_order("rev_1").await.unwrap().is_none());

        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 25_000_000);
        let balances = db.list_currency_balances(user.id).await.unwrap();
        assert_eq!(balances, vec![("eur".to_string(), 25_000_000, 25_000_000)]);
        let rows = db.list_transactions_for_reference(&format!("fiat-{}", created.public_id)).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].currency.as_str(), rows[0].amount_milli, rows[0].transaction_type.as_str()), ("eur", 25_000_000, "deposit"));
    }

    #[tokio::test]
    async fn concurrent_confirmations_credit_once() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        order(&db, user.id, "rev_2").await;
        let attempts: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                tokio::spawn(async move { db.complete_fiat_order("rev_2").await.unwrap().is_some() })
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
    async fn failed_and_cancelled_orders_credit_nothing_even_if_completed_later() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        order(&db, user.id, "rev_f").await;
        order(&db, user.id, "rev_c").await;
        assert!(db.close_fiat_order("rev_f", FAILED, Some("declined")).await.unwrap().is_some());
        assert!(db.close_fiat_order("rev_c", CANCELLED, None).await.unwrap().is_some());
        assert!(db.close_fiat_order("rev_c", FAILED, None).await.unwrap().is_none());
        assert!(db.complete_fiat_order("rev_f").await.unwrap().is_none());
        assert!(db.complete_fiat_order("rev_c").await.unwrap().is_none());
        assert_eq!(db.get_currency_balance(user.id, "eur").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn stale_pending_lists_only_old_pending_orders_with_an_external_id() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let old = order(&db, user.id, "rev_old").await;
        order(&db, user.id, "rev_new").await;
        db.execute_raw(&format!("UPDATE fiat_orders SET created_at = UTC_TIMESTAMP(3) - INTERVAL 1 HOUR WHERE id = {}", old.id))
            .await
            .unwrap();
        let stale = db.stale_pending_fiat_orders(600, 50).await.unwrap();
        assert_eq!(stale.iter().map(|o| o.external_id.clone().unwrap()).collect::<Vec<_>>(), vec!["rev_old"]);
    }
}
