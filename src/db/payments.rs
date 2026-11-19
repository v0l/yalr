use super::{BalanceTransactionRow, Database, UserBalanceRow, UserBalanceWithUsername};
use sqlx::{MySql, Transaction};

pub struct LedgerEntry<'a> {
    pub user_id: i64,
    pub amount_msat: i64,
    pub tx_type: &'a str,
    pub reference_id: &'a str,
    pub metadata: Option<&'a str>,
}

pub enum Debit {
    Done(i64),
    Insufficient { available: i64 },
}

async fn balance_in(tx: &mut Transaction<'_, MySql>, user_id: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT balance_msat FROM user_balances WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
}

async fn record(tx: &mut Transaction<'_, MySql>, entry: &LedgerEntry<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id, metadata) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(entry.user_id)
    .bind(entry.amount_msat)
    .bind(entry.tx_type)
    .bind(entry.reference_id)
    .bind(entry.metadata)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Database {
    pub async fn apply_balance_change(&self, entry: LedgerEntry<'_>, deposited_msat: i64) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO user_balances (user_id, balance_msat, lifetime_deposited_msat) VALUES (?, ?, ?) \
             ON DUPLICATE KEY UPDATE balance_msat = balance_msat + VALUES(balance_msat), \
             lifetime_deposited_msat = lifetime_deposited_msat + VALUES(lifetime_deposited_msat), \
             updated_at = CURRENT_TIMESTAMP(3)",
        )
        .bind(entry.user_id)
        .bind(entry.amount_msat)
        .bind(deposited_msat)
        .execute(&mut *tx)
        .await?;
        let balance = balance_in(&mut tx, entry.user_id).await?;
        record(&mut tx, &entry).await?;
        tx.commit().await?;
        Ok(balance)
    }

    pub async fn debit_balance(&self, entry: LedgerEntry<'_>) -> Result<Debit, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE user_balances SET balance_msat = balance_msat + ?, updated_at = CURRENT_TIMESTAMP(3) \
             WHERE user_id = ? AND balance_msat >= ?",
        )
        .bind(entry.amount_msat)
        .bind(entry.user_id)
        .bind(-entry.amount_msat)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            tx.rollback().await?;
            let available = self.get_user_balance(entry.user_id).await?.map_or(0, |b| b.balance_msat);
            return Ok(Debit::Insufficient { available });
        }
        let balance = balance_in(&mut tx, entry.user_id).await?;
        record(&mut tx, &entry).await?;
        tx.commit().await?;
        Ok(Debit::Done(balance))
    }

    pub async fn get_user_balance(&self, user_id: i64) -> Result<Option<UserBalanceRow>, sqlx::Error> {
        sqlx::query_as::<_, UserBalanceRow>(
            "SELECT id, user_id, balance_msat, lifetime_deposited_msat, created_at, updated_at \
             FROM user_balances WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_user_transactions(&self, user_id: i64, limit: i64) -> Result<Vec<BalanceTransactionRow>, sqlx::Error> {
        sqlx::query_as::<_, BalanceTransactionRow>(
            "SELECT id, user_id, amount_msat, transaction_type, reference_id, metadata, currency, amount_milli, created_at \
             FROM balance_transactions WHERE user_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_all_user_balances(&self, limit: i64) -> Result<Vec<UserBalanceWithUsername>, sqlx::Error> {
        sqlx::query_as::<_, UserBalanceWithUsername>(
            "SELECT ub.id, ub.user_id, ub.balance_msat, ub.lifetime_deposited_msat, ub.created_at, ub.updated_at, \
             COALESCE(NULLIF(u.display_name, ''), u.username, u.external_id, CONCAT('user-', ub.user_id)) AS username \
             FROM user_balances ub LEFT JOIN users u ON ub.user_id = u.id \
             ORDER BY ub.balance_msat DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_user_balance_with_username(&self, user_id: i64) -> Result<Vec<UserBalanceWithUsername>, sqlx::Error> {
        sqlx::query_as::<_, UserBalanceWithUsername>(
            "SELECT ub.id, ub.user_id, ub.balance_msat, ub.lifetime_deposited_msat, ub.created_at, ub.updated_at, \
             COALESCE(NULLIF(u.display_name, ''), u.username, u.external_id, CONCAT('user-', ub.user_id)) AS username \
             FROM user_balances ub LEFT JOIN users u ON ub.user_id = u.id WHERE ub.user_id = ?",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_all_transactions(&self, limit: i64) -> Result<Vec<BalanceTransactionRow>, sqlx::Error> {
        sqlx::query_as::<_, BalanceTransactionRow>(
            "SELECT id, user_id, amount_msat, transaction_type, reference_id, metadata, currency, amount_milli, created_at \
             FROM balance_transactions ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }
}
