use super::currency::{add, record, SATS};
use super::{Database, Timestamp};
use sqlx::{MySql, Transaction};

pub const REQUESTED: &str = "requested";
pub const HELD: &str = "held";
pub const SENT: &str = "sent";
pub const COMPLETED: &str = "completed";
pub const FAILED: &str = "failed";
pub const CANCELLED: &str = "cancelled";

pub const LIGHTNING: &str = "lightning";
pub const REVOLUT_LINK: &str = "revolut_link";
pub const REVOLUT_BANK: &str = "revolut_bank";

const COLUMNS: &str = "id, user_id, currency, amount, fee, method, destination, status, external_id, \
                       failure_reason, created_at, updated_at, completed_at";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PayoutRow {
    pub id: i64,
    pub user_id: i64,
    pub currency: String,
    pub amount: i64,
    pub fee: i64,
    pub method: String,
    pub destination: String,
    pub status: String,
    pub external_id: Option<String>,
    pub failure_reason: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
}

#[derive(Debug, Clone)]
pub struct PayoutRules {
    pub min_amount: i64,
    pub daily_cap: i64,
    pub hold_days: i64,
    pub approval_threshold: i64,
    pub fee_ppm: i64,
    pub fee_flat: i64,
}

pub struct NewPayout<'a> {
    pub user_id: i64,
    pub currency: &'a str,
    pub amount: i64,
    pub method: &'a str,
    pub destination: &'a str,
    pub external_id: Option<&'a str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PayoutError {
    BelowMinimum { min: i64 },
    OverDailyCap { remaining: i64 },
    NotWithdrawable { withdrawable: i64 },
    Duplicate,
    WrongState,
    NotFound,
    Db(String),
}

impl From<sqlx::Error> for PayoutError {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => PayoutError::Duplicate,
            _ => PayoutError::Db(e.to_string()),
        }
    }
}

fn amount_column(currency: &str) -> &'static str {
    if currency == SATS { "amount_msat" } else { "amount_milli" }
}

fn balance_query(currency: &str) -> &'static str {
    if currency == SATS {
        "SELECT balance_msat FROM user_balances WHERE user_id = ? FOR UPDATE"
    } else {
        "SELECT balance_milli FROM user_currency_balances WHERE user_id = ? AND currency = ? FOR UPDATE"
    }
}

async fn locked_balance(tx: &mut Transaction<'_, MySql>, user_id: i64, currency: &str) -> Result<i64, sqlx::Error> {
    let query = sqlx::query_scalar(balance_query(currency)).bind(user_id);
    let query = if currency == SATS { query } else { query.bind(currency) };
    Ok(query.fetch_optional(&mut **tx).await?.unwrap_or(0))
}

async fn withdrawable_in(
    tx: &mut Transaction<'_, MySql>,
    user_id: i64,
    currency: &str,
    hold_days: i64,
) -> Result<i64, sqlx::Error> {
    let balance = locked_balance(tx, user_id, currency).await?;
    let earned: i64 = sqlx::query_scalar(&format!(
        "SELECT CAST(COALESCE(SUM({col}), 0) AS SIGNED) FROM balance_transactions \
         WHERE user_id = ? AND currency = ? AND transaction_type = 'provider_earning' \
         AND created_at <= CURRENT_TIMESTAMP(3) - INTERVAL ? DAY",
        col = amount_column(currency)
    ))
    .bind(user_id)
    .bind(currency)
    .bind(hold_days)
    .fetch_one(&mut **tx)
    .await?;
    let committed: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(amount), 0) AS SIGNED) FROM payouts \
         WHERE user_id = ? AND currency = ? AND status IN ('requested', 'held', 'sent', 'completed')",
    )
    .bind(user_id)
    .bind(currency)
    .fetch_one(&mut **tx)
    .await?;
    Ok(balance.min(earned - committed).max(0))
}

async fn paid_today(tx: &mut Transaction<'_, MySql>, user_id: i64, currency: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(amount), 0) AS SIGNED) FROM payouts \
         WHERE user_id = ? AND currency = ? AND status IN ('requested', 'held', 'sent', 'completed') \
         AND created_at > CURRENT_TIMESTAMP(3) - INTERVAL 1 DAY",
    )
    .bind(user_id)
    .bind(currency)
    .fetch_one(&mut **tx)
    .await
}

async fn has_prior_payout(tx: &mut Transaction<'_, MySql>, user_id: i64) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payouts WHERE user_id = ? AND status IN ('sent', 'completed')",
    )
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(count > 0)
}

async fn transition(
    tx: &mut Transaction<'_, MySql>,
    id: i64,
    from: &[&str],
    to: &str,
    reason: Option<&str>,
) -> Result<PayoutRow, PayoutError> {
    let row = fetch_in(tx, id).await?.ok_or(PayoutError::NotFound)?;
    if !from.contains(&row.status.as_str()) {
        return Err(PayoutError::WrongState);
    }
    let done = sqlx::query(
        "UPDATE payouts SET status = ?, failure_reason = COALESCE(?, failure_reason), \
         completed_at = IF(? = 'completed', CURRENT_TIMESTAMP(3), completed_at) WHERE id = ? AND status = ?",
    )
    .bind(to)
    .bind(reason)
    .bind(to)
    .bind(id)
    .bind(&row.status)
    .execute(&mut **tx)
    .await?;
    if done.rows_affected() == 0 {
        return Err(PayoutError::WrongState);
    }
    Ok(PayoutRow { status: to.to_string(), ..row })
}

async fn fetch_in(tx: &mut Transaction<'_, MySql>, id: i64) -> Result<Option<PayoutRow>, sqlx::Error> {
    sqlx::query_as(&format!("SELECT {COLUMNS} FROM payouts WHERE id = ? FOR UPDATE"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
}

async fn hold(tx: &mut Transaction<'_, MySql>, p: &PayoutRow) -> Result<(), PayoutError> {
    if locked_balance(tx, p.user_id, &p.currency).await? < p.amount {
        return Err(PayoutError::NotWithdrawable { withdrawable: 0 });
    }
    add(tx, p.user_id, &p.currency, -p.amount, 0).await?;
    record(tx, p.user_id, &p.currency, -p.amount, "payout_hold", &reference(p.id), None).await?;
    Ok(())
}

async fn release(tx: &mut Transaction<'_, MySql>, p: &PayoutRow) -> Result<(), PayoutError> {
    add(tx, p.user_id, &p.currency, p.amount, 0).await?;
    record(tx, p.user_id, &p.currency, p.amount, "payout_release", &reference(p.id), None).await?;
    Ok(())
}

fn reference(id: i64) -> String {
    format!("payout:{id}")
}

impl PayoutRules {
    pub fn fee_for(&self, amount: i64) -> i64 {
        (super::fee_for(amount, self.fee_ppm) + self.fee_flat).min(amount)
    }
}

impl Database {
    pub async fn withdrawable(&self, user_id: i64, currency: &str, hold_days: i64) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let value = withdrawable_in(&mut tx, user_id, currency, hold_days).await?;
        tx.rollback().await?;
        Ok(value)
    }

    pub async fn request_payout(&self, new: NewPayout<'_>, rules: &PayoutRules) -> Result<PayoutRow, PayoutError> {
        if new.amount < rules.min_amount {
            return Err(PayoutError::BelowMinimum { min: rules.min_amount });
        }
        let mut tx = self.pool.begin().await?;
        let withdrawable = withdrawable_in(&mut tx, new.user_id, new.currency, rules.hold_days).await?;
        if new.amount > withdrawable {
            return Err(PayoutError::NotWithdrawable { withdrawable });
        }
        let remaining = rules.daily_cap - paid_today(&mut tx, new.user_id, new.currency).await?;
        if new.amount > remaining {
            return Err(PayoutError::OverDailyCap { remaining: remaining.max(0) });
        }
        let needs_approval =
            new.amount > rules.approval_threshold || !has_prior_payout(&mut tx, new.user_id).await?;
        let status = if needs_approval { REQUESTED } else { HELD };
        let id = sqlx::query(
            "INSERT INTO payouts (user_id, currency, amount, fee, method, destination, status, external_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new.user_id)
        .bind(new.currency)
        .bind(new.amount)
        .bind(rules.fee_for(new.amount))
        .bind(new.method)
        .bind(new.destination)
        .bind(status)
        .bind(new.external_id)
        .execute(&mut *tx)
        .await?
        .last_insert_id() as i64;
        let row = fetch_in(&mut tx, id).await?.ok_or(PayoutError::NotFound)?;
        if !needs_approval {
            hold(&mut tx, &row).await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    pub async fn approve_payout(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[REQUESTED], HELD, None).await?;
        hold(&mut tx, &row).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn reject_payout(&self, id: i64, reason: &str) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[REQUESTED], CANCELLED, Some(reason)).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn mark_payout_sent(&self, id: i64, external_id: &str) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[HELD], SENT, None).await?;
        sqlx::query("UPDATE payouts SET external_id = ? WHERE id = ?")
            .bind(external_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(PayoutRow { external_id: Some(external_id.to_string()), ..row })
    }

    pub async fn complete_payout(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[SENT], COMPLETED, None).await?;
        record(&mut tx, row.user_id, &row.currency, 0, "payout_complete", &reference(id), None).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn fail_payout(&self, id: i64, reason: &str) -> Result<PayoutRow, PayoutError> {
        self.release_payout(id, FAILED, Some(reason)).await
    }

    pub async fn cancel_payout(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        self.release_payout(id, CANCELLED, None).await
    }

    async fn release_payout(&self, id: i64, to: &str, reason: Option<&str>) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[HELD, SENT], to, reason).await?;
        release(&mut tx, &row).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn retry_payout(&self, id: i64) -> Result<PayoutRow, PayoutError> {
        let mut tx = self.pool.begin().await?;
        let row = transition(&mut tx, id, &[FAILED], REQUESTED, None).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub async fn get_payout(&self, id: i64) -> Result<Option<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM payouts WHERE id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_payout_by_external(&self, method: &str, external_id: &str) -> Result<Option<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM payouts WHERE method = ? AND external_id = ?"))
            .bind(method)
            .bind(external_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_user_payouts(&self, user_id: i64) -> Result<Vec<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM payouts WHERE user_id = ? ORDER BY id DESC LIMIT 200"))
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn list_payouts(&self, status: Option<&str>) -> Result<Vec<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM payouts WHERE (? IS NULL OR status = ?) ORDER BY id DESC LIMIT 500"
        ))
        .bind(status)
        .bind(status)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn stale_sent_payouts(&self, method: &str, minutes: i64) -> Result<Vec<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM payouts WHERE method = ? AND status = 'sent' \
             AND updated_at < CURRENT_TIMESTAMP(3) - INTERVAL ? MINUTE ORDER BY id LIMIT 100"
        ))
        .bind(method)
        .bind(minutes)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn held_payouts(&self, method: &str) -> Result<Vec<PayoutRow>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM payouts WHERE method = ? AND status = 'held' ORDER BY id LIMIT 100"
        ))
        .bind(method)
        .fetch_all(&self.pool)
        .await
    }
}
