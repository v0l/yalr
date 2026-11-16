use super::{BalanceTransactionRow, Database};
use sqlx::{MySql, Transaction};

pub const SATS: &str = "sats";

pub struct Settlement<'a> {
    pub consumer_id: i64,
    pub owner_id: Option<i64>,
    pub currency: &'a str,
    pub amount: i64,
    pub fee_ppm: i64,
    pub reference_id: &'a str,
    pub model: Option<&'a str>,
    pub prefill_credit: i64,
    pub failover: Option<FailoverCharge<'a>>,
}

pub struct FailoverCharge<'a> {
    pub failed_provider: &'a str,
    pub failed_owner_id: Option<i64>,
    pub served_provider: &'a str,
    pub penalty: i64,
    pub expected_cached_tokens: u32,
    pub reason: &'a str,
}

#[derive(serde::Serialize)]
struct PenaltyMetadata<'a> {
    failed_provider: &'a str,
    served_provider: &'a str,
    reason: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Settled {
    pub debited: i64,
    pub owner_credit: i64,
    pub fee: i64,
    pub penalty: i64,
    pub platform_absorbed: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FailoverPenaltyTotal {
    pub day: String,
    pub failed_provider: String,
    pub currency: String,
    pub requests: i64,
    pub penalty_milli: i64,
    pub platform_milli: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct FailoverPenaltyRow {
    pub request_id: String,
    pub failed_provider: String,
    pub served_provider: String,
    pub currency: String,
    pub penalty_milli: i64,
    pub reason: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub fn normalize_currency(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_lowercase();
    let valid = (3..=8).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase());
    valid.then_some(code)
}

pub fn milli_per_unit(currency: &str) -> i64 {
    if currency == SATS { 1_000 } else { 100_000 }
}

pub fn fee_for(amount: i64, fee_ppm: i64) -> i64 {
    ((amount.max(0) as i128 * fee_ppm.clamp(0, 1_000_000) as i128) / 1_000_000) as i64
}

pub(super) async fn add(
    tx: &mut Transaction<'_, MySql>,
    user_id: i64,
    currency: &str,
    delta: i64,
    deposited: i64,
) -> Result<i64, sqlx::Error> {
    if currency == SATS {
        sqlx::query(
            "INSERT INTO user_balances (user_id, balance_msat, lifetime_deposited_msat) VALUES (?, ?, ?) \
             ON DUPLICATE KEY UPDATE balance_msat = balance_msat + VALUES(balance_msat), \
             lifetime_deposited_msat = lifetime_deposited_msat + VALUES(lifetime_deposited_msat), \
             updated_at = CURRENT_TIMESTAMP(3)",
        )
        .bind(user_id)
        .bind(delta)
        .bind(deposited)
        .execute(&mut **tx)
        .await?;
        return sqlx::query_scalar("SELECT balance_msat FROM user_balances WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await;
    }
    sqlx::query(
        "INSERT INTO user_currency_balances (user_id, currency, balance_milli, lifetime_deposited_milli) \
         VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE balance_milli = balance_milli + VALUES(balance_milli), \
         lifetime_deposited_milli = lifetime_deposited_milli + VALUES(lifetime_deposited_milli), \
         updated_at = CURRENT_TIMESTAMP(3)",
    )
    .bind(user_id)
    .bind(currency)
    .bind(delta)
    .bind(deposited)
    .execute(&mut **tx)
    .await?;
    sqlx::query_scalar("SELECT balance_milli FROM user_currency_balances WHERE user_id = ? AND currency = ?")
        .bind(user_id)
        .bind(currency)
        .fetch_one(&mut **tx)
        .await
}

pub(super) async fn record(
    tx: &mut Transaction<'_, MySql>,
    user_id: i64,
    currency: &str,
    delta: i64,
    tx_type: &str,
    reference_id: &str,
    metadata: Option<&str>,
) -> Result<(), sqlx::Error> {
    let (msat, milli) = if currency == SATS { (delta, 0) } else { (0, delta) };
    sqlx::query(
        "INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id, metadata, currency, amount_milli) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(msat)
    .bind(tx_type)
    .bind(reference_id)
    .bind(metadata)
    .bind(currency)
    .bind(milli)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Database {
    pub async fn get_currency_balance(&self, user_id: i64, currency: &str) -> Result<i64, sqlx::Error> {
        if currency == SATS {
            return Ok(self.get_user_balance(user_id).await?.map_or(0, |b| b.balance_msat));
        }
        let balance = sqlx::query_scalar(
            "SELECT balance_milli FROM user_currency_balances WHERE user_id = ? AND currency = ?",
        )
        .bind(user_id)
        .bind(currency)
        .fetch_optional(&self.pool)
        .await?;
        Ok(balance.unwrap_or(0))
    }

    pub async fn list_currency_balances(&self, user_id: i64) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT currency, balance_milli, lifetime_deposited_milli FROM user_currency_balances \
             WHERE user_id = ? ORDER BY currency",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_currency_balances_for_users(&self, user_ids: &[i64]) -> Result<Vec<(i64, String, i64, i64)>, sqlx::Error> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; user_ids.len()].join(",");
        let sql = format!(
            "SELECT user_id, currency, balance_milli, lifetime_deposited_milli FROM user_currency_balances \
             WHERE user_id IN ({marks}) ORDER BY user_id, currency"
        );
        let mut query = sqlx::query_as(&sql);
        for id in user_ids {
            query = query.bind(id);
        }
        query.fetch_all(&self.pool).await
    }

    pub async fn credit_currency(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: &str,
        reference_id: &str,
        deposited: i64,
    ) -> Result<i64, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let balance = add(&mut tx, user_id, currency, amount, deposited).await?;
        record(&mut tx, user_id, currency, amount, tx_type, reference_id, None).await?;
        tx.commit().await?;
        Ok(balance)
    }

    pub async fn debit_currency(
        &self,
        user_id: i64,
        currency: &str,
        amount: i64,
        tx_type: &str,
        reference_id: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let updated = if currency == SATS {
            sqlx::query("UPDATE user_balances SET balance_msat = balance_msat - ?, updated_at = CURRENT_TIMESTAMP(3) WHERE user_id = ? AND balance_msat >= ?")
        } else {
            sqlx::query("UPDATE user_currency_balances SET balance_milli = balance_milli - ?, updated_at = CURRENT_TIMESTAMP(3) WHERE user_id = ? AND balance_milli >= ? AND currency = ?")
        };
        let updated = updated.bind(amount).bind(user_id).bind(amount);
        let updated = if currency == SATS { updated } else { updated.bind(currency) };
        if updated.execute(&mut *tx).await?.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        let balance = add(&mut tx, user_id, currency, 0, 0).await?;
        record(&mut tx, user_id, currency, -amount, tx_type, reference_id, None).await?;
        tx.commit().await?;
        Ok(Some(balance))
    }

    pub async fn known_currencies(&self) -> Result<Vec<String>, sqlx::Error> {
        let mut codes: Vec<String> = sqlx::query_scalar(
            "SELECT currency FROM model_pricing UNION SELECT currency FROM user_currency_balances",
        )
        .fetch_all(&self.pool)
        .await?;
        codes.extend([SATS.to_string(), "usd".to_string()]);
        codes.sort_by(|a, b| (a != SATS, a).cmp(&(b != SATS, b)));
        codes.dedup();
        Ok(codes)
    }

    pub async fn settle_usage(&self, s: Settlement<'_>) -> Result<Settled, sqlx::Error> {
        if s.amount <= 0 || s.owner_id == Some(s.consumer_id) {
            return Ok(Settled::default());
        }
        let mut tx = self.pool.begin().await?;
        let debited = (s.amount - s.prefill_credit.clamp(0, s.amount)).max(0);
        if debited > 0 {
            add(&mut tx, s.consumer_id, s.currency, -debited, 0).await?;
            record(&mut tx, s.consumer_id, s.currency, -debited, "usage", s.reference_id, s.model).await?;
        }
        let mut settled = Settled { debited, ..Settled::default() };
        if let Some(owner) = s.owner_id {
            settled.fee = fee_for(s.amount, s.fee_ppm);
            settled.owner_credit = s.amount - settled.fee;
            if settled.owner_credit > 0 {
                add(&mut tx, owner, s.currency, settled.owner_credit, 0).await?;
                record(&mut tx, owner, s.currency, settled.owner_credit, "provider_earning", s.reference_id, s.model)
                    .await?;
            }
        }
        if let Some(f) = &s.failover {
            let penalty = f.penalty.clamp(0, s.prefill_credit);
            if let (Some(owner), true) = (f.failed_owner_id, penalty > 0) {
                let metadata = serde_json::to_string(&PenaltyMetadata {
                    failed_provider: f.failed_provider,
                    served_provider: f.served_provider,
                    reason: f.reason,
                })
                .ok();
                add(&mut tx, owner, s.currency, -penalty, 0).await?;
                record(&mut tx, owner, s.currency, -penalty, "failover_penalty", s.reference_id, metadata.as_deref())
                    .await?;
            }
            let applied = if f.failed_owner_id.is_some() { penalty } else { 0 };
            sqlx::query(
                "INSERT INTO failover_penalties (request_id, failed_provider, failed_owner_id, served_provider, \
                 currency, gap_milli, penalty_milli, platform_milli, expected_cached_tokens, reason) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(s.reference_id)
            .bind(f.failed_provider)
            .bind(f.failed_owner_id)
            .bind(f.served_provider)
            .bind(s.currency)
            .bind(s.prefill_credit)
            .bind(applied)
            .bind(s.prefill_credit - applied)
            .bind(f.expected_cached_tokens as i64)
            .bind(f.reason)
            .execute(&mut *tx)
            .await?;
            settled.penalty = applied;
            settled.platform_absorbed = s.prefill_credit - applied;
        }
        tx.commit().await?;
        Ok(settled)
    }

    pub async fn failover_penalty_totals(&self, since_ms: i64) -> Result<Vec<FailoverPenaltyTotal>, sqlx::Error> {
        sqlx::query_as::<_, FailoverPenaltyTotal>(
            "SELECT DATE_FORMAT(created_at, '%Y-%m-%d') AS day, failed_provider, currency, \
             CAST(COUNT(*) AS SIGNED) AS requests, \
             CAST(SUM(penalty_milli) AS SIGNED) AS penalty_milli, \
             CAST(SUM(platform_milli) AS SIGNED) AS platform_milli \
             FROM failover_penalties WHERE created_at >= ? \
             GROUP BY day, failed_provider, currency ORDER BY day DESC, failed_provider",
        )
        .bind(chrono::DateTime::from_timestamp_millis(since_ms).unwrap_or_default())
        .fetch_all(&self.pool)
        .await
    }

    pub async fn failover_penalties_for_owner(&self, owner_id: i64, limit: i64) -> Result<Vec<FailoverPenaltyRow>, sqlx::Error> {
        sqlx::query_as::<_, FailoverPenaltyRow>(
            "SELECT request_id, failed_provider, served_provider, currency, penalty_milli, reason, created_at \
             FROM failover_penalties WHERE failed_owner_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(owner_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_transactions_for_reference(&self, reference_id: &str) -> Result<Vec<BalanceTransactionRow>, sqlx::Error> {
        sqlx::query_as::<_, BalanceTransactionRow>(
            "SELECT id, user_id, amount_msat, transaction_type, reference_id, metadata, currency, amount_milli, created_at \
             FROM balance_transactions WHERE reference_id = ? ORDER BY id",
        )
        .bind(reference_id)
        .fetch_all(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_codes_are_normalised() {
        assert_eq!(normalize_currency(" USD "), Some("usd".to_string()));
        assert_eq!(normalize_currency("eur"), Some("eur".to_string()));
        assert_eq!(normalize_currency("us"), None);
        assert_eq!(normalize_currency("u$d"), None);
    }

    #[tokio::test]
    async fn credit_and_debit_share_one_path_for_every_currency() {
        let db = crate::db::Database::test().await;
        let user = db.add_test_user("u", false).await;
        for (currency, expected_balance) in [("sats", 700), ("usd", 700)] {
            db.credit_currency(user.id, currency, 1_000, "admin_credit", "c", 1_000).await.unwrap();
            assert_eq!(db.debit_currency(user.id, currency, 300, "admin_debit", "d").await.unwrap(), Some(expected_balance));
            assert_eq!(db.debit_currency(user.id, currency, 701, "admin_debit", "d2").await.unwrap(), None);
            assert_eq!(db.get_currency_balance(user.id, currency).await.unwrap(), expected_balance);
        }
    }

    #[tokio::test]
    async fn known_currencies_lists_sats_first_then_every_one_in_use() {
        let db = crate::db::Database::test().await;
        let user = db.add_test_user("u", false).await;
        db.credit_currency(user.id, "eur", 5, "admin_credit", "c", 5).await.unwrap();
        assert_eq!(db.known_currencies().await.unwrap(), vec!["sats", "eur", "usd"]);
    }

    #[test]
    fn fee_is_parts_per_million_rounded_down() {
        assert_eq!(fee_for(1_000_000, 50_000), 50_000);
        assert_eq!(fee_for(19, 500_000), 9);
        assert_eq!(fee_for(1000, 0), 0);
        assert_eq!(fee_for(1000, 2_000_000), 1000);
    }
}
