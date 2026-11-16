use super::Database;

const COLUMNS: &str = "id, user_id, provider, external_customer_id, external_id, card_brand, card_last_four, \
    exp_month, exp_year, is_default, created_at";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PaymentMethod {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub external_customer_id: String,
    pub external_id: String,
    pub card_brand: Option<String>,
    pub card_last_four: Option<String>,
    pub exp_month: Option<u16>,
    pub exp_year: Option<u16>,
    pub is_default: bool,
    pub created_at: super::Timestamp,
}

pub struct NewPaymentMethod<'a> {
    pub user_id: i64,
    pub external_customer_id_encrypted: &'a str,
    pub external_id: &'a str,
    pub card_brand: Option<&'a str>,
    pub card_last_four: Option<&'a str>,
    pub exp_month: Option<u16>,
    pub exp_year: Option<u16>,
}

impl Database {
    pub async fn add_payment_method(&self, new: NewPaymentMethod<'_>) -> Result<PaymentMethod, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let has_default: Option<i64> = sqlx::query_scalar("SELECT id FROM user_payment_methods WHERE user_id = ? AND is_default = 1 FOR UPDATE")
            .bind(new.user_id)
            .fetch_optional(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT IGNORE INTO user_payment_methods \
             (user_id, external_customer_id, external_id, card_brand, card_last_four, exp_month, exp_year, is_default) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(new.user_id)
        .bind(new.external_customer_id_encrypted)
        .bind(new.external_id)
        .bind(new.card_brand)
        .bind(new.card_last_four)
        .bind(new.exp_month)
        .bind(new.exp_year)
        .bind(has_default.is_none())
        .execute(&mut *tx)
        .await?;
        let method = sqlx::query_as(&format!("SELECT {COLUMNS} FROM user_payment_methods WHERE provider = 'revolut' AND external_id = ?"))
            .bind(new.external_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(method)
    }

    pub async fn list_payment_methods(&self, user_id: i64) -> Result<Vec<PaymentMethod>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM user_payment_methods WHERE user_id = ? ORDER BY is_default DESC, id"))
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn payment_method(&self, user_id: i64, id: i64) -> Result<Option<PaymentMethod>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM user_payment_methods WHERE user_id = ? AND id = ?"))
            .bind(user_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn user_customer_ids_encrypted(&self, user_id: i64) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT external_customer_id FROM user_payment_methods WHERE user_id = ? ORDER BY id LIMIT 1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn delete_payment_method(&self, user_id: i64, id: i64) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let was_default: Option<bool> = sqlx::query_scalar("SELECT is_default FROM user_payment_methods WHERE user_id = ? AND id = ? FOR UPDATE")
            .bind(user_id)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(was_default) = was_default else {
            tx.rollback().await?;
            return Ok(false);
        };
        sqlx::query("DELETE FROM user_payment_methods WHERE user_id = ? AND id = ?")
            .bind(user_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        if was_default {
            sqlx::query("UPDATE user_payment_methods SET is_default = 1 WHERE user_id = ? ORDER BY id LIMIT 1")
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn set_default_payment_method(&self, user_id: i64, id: i64) -> Result<bool, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM user_payment_methods WHERE user_id = ? AND id = ? FOR UPDATE")
            .bind(user_id)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if found.is_none() {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query("UPDATE user_payment_methods SET is_default = (id = ?) WHERE user_id = ?")
            .bind(id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card<'a>(user_id: i64, external_id: &'a str) -> NewPaymentMethod<'a> {
        NewPaymentMethod {
            user_id,
            external_customer_id_encrypted: "sealed",
            external_id,
            card_brand: Some("visa"),
            card_last_four: Some("4242"),
            exp_month: Some(12),
            exp_year: Some(2030),
        }
    }

    #[tokio::test]
    async fn first_card_is_default_and_saving_twice_keeps_one_row() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let a = db.add_payment_method(card(user.id, "pm_a")).await.unwrap();
        let again = db.add_payment_method(card(user.id, "pm_a")).await.unwrap();
        let b = db.add_payment_method(card(user.id, "pm_b")).await.unwrap();
        assert!(a.is_default && !b.is_default);
        assert_eq!(a.id, again.id);
        assert_eq!(db.list_payment_methods(user.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn default_moves_on_set_and_on_delete_and_other_users_cannot_touch_it() {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let other = db.add_test_user("o", false).await;
        let a = db.add_payment_method(card(user.id, "pm_a")).await.unwrap();
        let b = db.add_payment_method(card(user.id, "pm_b")).await.unwrap();
        assert!(!db.set_default_payment_method(other.id, b.id).await.unwrap());
        assert!(!db.delete_payment_method(other.id, b.id).await.unwrap());
        assert!(db.set_default_payment_method(user.id, b.id).await.unwrap());
        assert_eq!(db.list_payment_methods(user.id).await.unwrap()[0].id, b.id);
        assert!(db.delete_payment_method(user.id, b.id).await.unwrap());
        let left = db.list_payment_methods(user.id).await.unwrap();
        assert_eq!((left.len(), left[0].id, left[0].is_default), (1, a.id, true));
    }
}
