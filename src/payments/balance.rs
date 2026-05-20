use std::sync::Arc;
use crate::db::{Database, BalanceTransactionRow};

/// Service for managing user balances with atomic credit/debit operations.
/// All amounts are in millisatoshis (msats).
pub struct BalanceService {
    db: Arc<Database>,
}

impl BalanceService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Get the current balance for a user. Returns 0 if no balance row exists yet.
    pub async fn get_balance(&self, user_id: i64) -> Result<i64, BalanceError> {
        let row = self.db.get_user_balance(user_id).await?;
        Ok(row.map(|b| b.balance_msat).unwrap_or(0))
    }

    /// Credit (add) to a user's balance. Creates the balance row if it doesn't exist.
    pub async fn credit(
        &self,
        user_id: i64,
        amount_msat: i64,
        tx_type: &str,
        reference_id: &str,
    ) -> Result<i64, BalanceError> {
        if amount_msat <= 0 {
            return Err(BalanceError::InvalidAmount(amount_msat));
        }

        // Use a DB transaction: upsert balance + insert transaction log
        let mut tx = self.db.pool.begin().await?;

        let new_balance = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO user_balances (user_id, balance_msat, lifetime_deposited_msat)
               VALUES (?, ?, ?)
               ON CONFLICT(user_id) DO UPDATE SET
                   balance_msat = balance_msat + ?,
                   lifetime_deposited_msat = lifetime_deposited_msat + ?,
                   updated_at = CURRENT_TIMESTAMP
               RETURNING balance_msat"#,
        )
        .bind(user_id)
        .bind(amount_msat)
        .bind(amount_msat)
        .bind(amount_msat)
        .bind(amount_msat)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id)
               VALUES (?, ?, ?, ?)"#,
        )
        .bind(user_id)
        .bind(amount_msat)
        .bind(tx_type)
        .bind(reference_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(new_balance)
    }

    /// Debit (subtract) from a user's balance. Fails if insufficient funds.
    pub async fn debit(
        &self,
        user_id: i64,
        amount_msat: i64,
        tx_type: &str,
        reference_id: &str,
    ) -> Result<i64, BalanceError> {
        if amount_msat <= 0 {
            return Err(BalanceError::InvalidAmount(amount_msat));
        }

        let negative = -amount_msat;
        let mut tx = self.db.pool.begin().await?;

        // Ensure balance row exists and check sufficiency atomically
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT balance_msat FROM user_balances WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);

        if current < amount_msat {
            return Err(BalanceError::InsufficientFunds {
                required: amount_msat,
                available: current,
            });
        }

        let new_balance = sqlx::query_scalar::<_, i64>(
            r#"UPDATE user_balances SET
                   balance_msat = balance_msat + ?,
                   updated_at = CURRENT_TIMESTAMP
               WHERE user_id = ?
               RETURNING balance_msat"#,
        )
        .bind(negative)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id)
               VALUES (?, ?, ?, ?)"#,
        )
        .bind(user_id)
        .bind(negative)
        .bind(tx_type)
        .bind(reference_id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(new_balance)
    }

    /// Get recent transactions for a user.
    pub async fn get_transactions(
        &self,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<BalanceTransactionRow>, BalanceError> {
        let rows = self.db.get_user_transactions(user_id, limit).await?;
        Ok(rows)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BalanceError {
    #[error("Invalid amount: {0}")]
    InvalidAmount(i64),

    #[error("Insufficient funds: required {required} msats, available {available} msats")]
    InsufficientFunds { required: i64, available: i64 },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("{0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (BalanceService, Arc<Database>) {
        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        // Create a user for the tests
        sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES ('test', 0, 0)")
            .execute(&db.pool)
            .await
            .unwrap();
        (BalanceService::new(db.clone()), db)
    }

    #[tokio::test]
    async fn test_get_balance_new_user() {
        let (svc, _) = setup().await;
        let balance = svc.get_balance(1).await.unwrap();
        assert_eq!(balance, 0);
    }

    #[tokio::test]
    async fn test_credit_creates_balance() {
        let (svc, _) = setup().await;
        let new_balance = svc.credit(1, 100_000, "deposit", "ref1").await.unwrap();
        assert_eq!(new_balance, 100_000);
    }

    #[tokio::test]
    async fn test_credit_accumulates() {
        let (svc, _) = setup().await;
        svc.credit(1, 100_000, "deposit", "ref1").await.unwrap();
        let new_balance = svc.credit(1, 50_000, "deposit", "ref2").await.unwrap();
        assert_eq!(new_balance, 150_000);
    }

    #[tokio::test]
    async fn test_debit_success() {
        let (svc, _) = setup().await;
        svc.credit(1, 100_000, "deposit", "ref1").await.unwrap();
        let new_balance = svc.debit(1, 30_000, "charge", "req1").await.unwrap();
        assert_eq!(new_balance, 70_000);
    }

    #[tokio::test]
    async fn test_debit_insufficient_funds() {
        let (svc, _) = setup().await;
        svc.credit(1, 10_000, "deposit", "ref1").await.unwrap();
        let result = svc.debit(1, 20_000, "charge", "req1").await;
        assert!(matches!(result, Err(BalanceError::InsufficientFunds { .. })));
    }

    #[tokio::test]
    async fn test_debit_invalid_amount() {
        let (svc, _) = setup().await;
        let result = svc.debit(1, 0, "charge", "req1").await;
        assert!(matches!(result, Err(BalanceError::InvalidAmount(0))));
    }

    #[tokio::test]
    async fn test_credit_invalid_amount() {
        let (svc, _) = setup().await;
        let result = svc.credit(1, -100, "deposit", "ref1").await;
        assert!(matches!(result, Err(BalanceError::InvalidAmount(-100))));
    }

    #[tokio::test]
    async fn test_get_transactions() {
        let (svc, _) = setup().await;
        svc.credit(1, 100_000, "deposit", "ref1").await.unwrap();
        svc.debit(1, 20_000, "charge", "req1").await.unwrap();
        svc.credit(1, 50_000, "deposit", "ref2").await.unwrap();

        let txs = svc.get_transactions(1, 10).await.unwrap();
        assert_eq!(txs.len(), 3);
        // Transactions returned newest first (ORDER BY created_at DESC, id DESC)
        assert_eq!(txs[0].amount_msat, 50_000);
        assert_eq!(txs[1].amount_msat, -20_000);
        assert_eq!(txs[2].amount_msat, 100_000);
    }
}
