use std::sync::Arc;
use crate::db::{BalanceTransactionRow, Database, Debit, LedgerEntry};

pub fn is_deposit(tx_type: &str) -> bool {
    !matches!(tx_type, "refund" | "refund_reversal")
}

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
        let deposited = if is_deposit(tx_type) { amount_msat } else { 0 };
        let entry = LedgerEntry { user_id, amount_msat, tx_type, reference_id, metadata: None };
        Ok(self.db.apply_balance_change(entry, deposited).await?)
    }

    pub async fn charge(
        &self,
        user_id: i64,
        amount_msat: i64,
        tx_type: &str,
        reference_id: &str,
        metadata: Option<&str>,
    ) -> Result<i64, BalanceError> {
        if amount_msat <= 0 {
            return Err(BalanceError::InvalidAmount(amount_msat));
        }
        let entry = LedgerEntry { user_id, amount_msat: -amount_msat, tx_type, reference_id, metadata };
        Ok(self.db.apply_balance_change(entry, 0).await?)
    }

    /// Debit (subtract) from a user's balance. Fails if insufficient funds.
    pub async fn debit(
        &self,
        user_id: i64,
        amount_msat: i64,
        tx_type: &str,
        reference_id: &str,
    ) -> Result<i64, BalanceError> {
        self.debit_with_metadata(user_id, amount_msat, tx_type, reference_id, None).await
    }

    pub async fn debit_with_metadata(
        &self,
        user_id: i64,
        amount_msat: i64,
        tx_type: &str,
        reference_id: &str,
        metadata: Option<&str>,
    ) -> Result<i64, BalanceError> {
        if amount_msat <= 0 {
            return Err(BalanceError::InvalidAmount(amount_msat));
        }
        let entry = LedgerEntry { user_id, amount_msat: -amount_msat, tx_type, reference_id, metadata };
        match self.db.debit_balance(entry).await? {
            Debit::Done(balance) => Ok(balance),
            Debit::Insufficient { available } => {
                Err(BalanceError::InsufficientFunds { required: amount_msat, available })
            }
        }
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
        let db = Arc::new(Database::test().await);
        // Create a user for the tests
        db.execute_raw("INSERT INTO users (username, user_type, is_admin) VALUES ('test', 0, 0)").await.unwrap();
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

    async fn lifetime(svc: &BalanceService, user_id: i64) -> i64 {
        svc.db.get_user_balance(user_id).await.unwrap().unwrap().lifetime_deposited_msat
    }

    #[tokio::test]
    async fn only_real_deposits_count_toward_lifetime() {
        let (svc, _) = setup().await;
        svc.credit(1, 100_000, "deposit", "a").await.unwrap();
        svc.credit(1, 6_000, "refund", "b").await.unwrap();
        svc.credit(1, 5_000, "refund_reversal", "c").await.unwrap();
        svc.credit(1, 2_000, "goodwill top-up", "d").await.unwrap();
        assert_eq!(lifetime(&svc, 1).await, 102_000);
        assert_eq!(svc.get_balance(1).await.unwrap(), 113_000);
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_debits_never_overdraw() {
        let (svc, _db) = setup().await;
        svc.credit(1, 10_000, "deposit", "seed").await.unwrap();
        let svc = Arc::new(svc);
        let debits = (0..40).map(|n| {
            let svc = svc.clone();
            tokio::spawn(async move { svc.debit(1, 1_000, "usage", &format!("d{n}")).await.is_ok() })
        });
        let succeeded = futures::future::join_all(debits).await.into_iter().filter(|r| *r.as_ref().unwrap()).count();
        assert_eq!(succeeded, 10);
        assert_eq!(svc.get_balance(1).await.unwrap(), 0);
        assert_eq!(svc.get_transactions(1, 100).await.unwrap().len(), 11);
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
