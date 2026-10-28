use super::{BalanceTransactionRow, Database, LightningInvoiceRow, NewLightningInvoice, UserBalanceRow, UserBalanceWithUsername};
use sqlx::{MySql, Transaction};

const INVOICE_COLUMNS: &str =
    "id, user_id, payment_hash, bolt11, amount_msat, amount_sats, status, created_at, expires_at, paid_at";

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
            "SELECT id, user_id, amount_msat, transaction_type, reference_id, metadata, created_at \
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
             COALESCE(u.username, u.external_id, CONCAT('user-', ub.user_id)) AS username \
             FROM user_balances ub LEFT JOIN users u ON ub.user_id = u.id \
             ORDER BY ub.balance_msat DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_all_transactions(&self, limit: i64) -> Result<Vec<BalanceTransactionRow>, sqlx::Error> {
        sqlx::query_as::<_, BalanceTransactionRow>(
            "SELECT id, user_id, amount_msat, transaction_type, reference_id, metadata, created_at \
             FROM balance_transactions ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_all_lightning_invoices(
        &self,
        user_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<LightningInvoiceRow>, sqlx::Error> {
        let mut query = sqlx::QueryBuilder::new(format!("SELECT {INVOICE_COLUMNS} FROM lightning_invoices"));
        if let Some(uid) = user_id {
            query.push(" WHERE user_id = ").push_bind(uid);
        }
        query.push(" ORDER BY created_at DESC, id DESC LIMIT ").push_bind(limit);
        query.build_query_as().fetch_all(&self.pool).await
    }

    pub async fn create_lightning_invoice(&self, invoice: NewLightningInvoice<'_>) -> Result<LightningInvoiceRow, sqlx::Error> {
        let id = sqlx::query(
            "INSERT INTO lightning_invoices (user_id, payment_hash, bolt11, amount_msat, amount_sats, status, expires_at) \
             VALUES (?, ?, ?, ?, ?, 'pending', UTC_TIMESTAMP(3) + INTERVAL ? SECOND)",
        )
        .bind(invoice.user_id)
        .bind(invoice.payment_hash)
        .bind(invoice.bolt11)
        .bind(invoice.amount_msat)
        .bind(invoice.amount_sats)
        .bind(invoice.expire_seconds.unwrap_or(3600))
        .execute(&self.pool)
        .await?
        .last_insert_id() as i64;
        self.lightning_invoice_by_id(id).await
    }

    async fn lightning_invoice_by_id(&self, id: i64) -> Result<LightningInvoiceRow, sqlx::Error> {
        sqlx::query_as::<_, LightningInvoiceRow>(&format!("SELECT {INVOICE_COLUMNS} FROM lightning_invoices WHERE id = ?"))
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_lightning_invoice_by_hash(&self, payment_hash: &str) -> Result<Option<LightningInvoiceRow>, sqlx::Error> {
        sqlx::query_as::<_, LightningInvoiceRow>(&format!(
            "SELECT {INVOICE_COLUMNS} FROM lightning_invoices WHERE payment_hash = ?"
        ))
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_invoice_by_bolt11(&self, bolt11: &str) -> Result<Option<LightningInvoiceRow>, sqlx::Error> {
        sqlx::query_as::<_, LightningInvoiceRow>(&format!(
            "SELECT {INVOICE_COLUMNS} FROM lightning_invoices WHERE bolt11 = ?"
        ))
        .bind(bolt11)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn update_lightning_invoice_status(
        &self,
        payment_hash: &str,
        status: &str,
    ) -> Result<Option<LightningInvoiceRow>, sqlx::Error> {
        let set_paid = if status == "paid" { ", paid_at = UTC_TIMESTAMP(3)" } else { "" };
        sqlx::query(&format!("UPDATE lightning_invoices SET status = ?{set_paid} WHERE payment_hash = ?"))
            .bind(status)
            .bind(payment_hash)
            .execute(&self.pool)
            .await?;
        self.get_lightning_invoice_by_hash(payment_hash).await
    }
}
