use super::Database;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PublicInvoice {
    pub payment_hash: String,
    pub purpose: String,
    pub public_id: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountInvoice {
    pub public_id: String,
    pub purpose: String,
    pub amount_sats: i64,
    pub status: String,
    pub created_at: super::Timestamp,
    pub paid_at: Option<super::Timestamp>,
}

impl Database {
    pub async fn server_secret(&self) -> Result<String, sqlx::Error> {
        sqlx::query("INSERT IGNORE INTO server_secret (id, secret) VALUES (1, ?)")
            .bind(random_hex())
            .execute(&self.pool)
            .await?;
        sqlx::query_scalar("SELECT secret FROM server_secret WHERE id = 1")
            .fetch_one(&self.pool)
            .await
    }

    pub async fn mark_public_invoice(
        &self,
        payment_hash: &str,
        purpose: &str,
        public_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE lightning_invoices SET purpose = ?, public_id = ? WHERE payment_hash = ?")
            .bind(purpose)
            .bind(public_id)
            .bind(payment_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_unpaid_anonymous_users(&self) -> Result<u64, sqlx::Error> {
        let done = sqlx::query(
            "DELETE FROM users WHERE created_at < UTC_TIMESTAMP(3) - INTERVAL 2 DAY AND created_at >= UTC_TIMESTAMP(3) - INTERVAL 3 DAY
             AND username LIKE 'anon_%'
             AND NOT EXISTS (SELECT 1 FROM lightning_invoices WHERE user_id = users.id AND status = 'paid')",
        )
        .execute(&self.pool)
        .await?;
        self.invalidate_auth_cache();
        Ok(done.rows_affected())
    }

    pub async fn list_account_invoices(&self, user_id: i64, limit: i64) -> Result<Vec<AccountInvoice>, sqlx::Error> {
        sqlx::query_as::<_, AccountInvoice>(
            "SELECT public_id, purpose, amount_sats, status, created_at, paid_at FROM lightning_invoices
             WHERE user_id = ? AND public_id IS NOT NULL ORDER BY id DESC LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn request_models(
        &self,
        user_id: i64,
        reservation_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, String>, sqlx::Error> {
        let ids: Vec<String> = reservation_ids.into_iter().filter(|r| r.starts_with("req-")).collect();
        if ids.is_empty() {
            return Ok(Default::default());
        }
        let marks = vec!["?"; ids.len()].join(",");
        let sql = format!(
            "SELECT reference_id, metadata FROM balance_transactions WHERE user_id = ? AND reference_id IN ({marks}) AND metadata IS NOT NULL"
        );
        let mut q = sqlx::query_as::<_, (String, String)>(&sql).bind(user_id);
        for id in &ids {
            q = q.bind(id);
        }
        Ok(q.fetch_all(&self.pool).await?.into_iter().collect())
    }

    pub async fn get_public_invoice(&self, public_id: &str) -> Result<Option<PublicInvoice>, sqlx::Error> {
        sqlx::query_as::<_, PublicInvoice>(
            "SELECT payment_hash, purpose, public_id FROM lightning_invoices WHERE public_id = ?",
        )
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn get_public_invoice_by_hash(&self, payment_hash: &str) -> Result<Option<PublicInvoice>, sqlx::Error> {
        sqlx::query_as::<_, PublicInvoice>(
            "SELECT payment_hash, purpose, public_id FROM lightning_invoices WHERE payment_hash = ? AND public_id IS NOT NULL",
        )
        .bind(payment_hash)
        .fetch_optional(&self.pool)
        .await
    }

}

pub fn random_hex() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prunes_only_old_unpaid_anonymous_users() {
        let db = Database::test().await;
        for (name, hours) in [("anon_old", 60), ("anon_new", 1), ("alice", 60)] {
            let user = db.add_test_user(name, false).await;
            db.execute_raw(&format!(
                "UPDATE users SET created_at = UTC_TIMESTAMP(3) - INTERVAL {hours} HOUR WHERE id = {}",
                user.id
            ))
            .await
            .unwrap();
        }
        assert_eq!(db.prune_unpaid_anonymous_users().await.unwrap(), 1);
        assert!(db.get_user_by_username("anon_old").await.unwrap().is_none());
        assert!(db.get_user_by_username("anon_new").await.unwrap().is_some());
        assert!(db.get_user_by_username("alice").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn server_secret_is_stable() {
        let db = Database::test().await;
        let first = db.server_secret().await.unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(db.server_secret().await.unwrap(), first);
    }
}
