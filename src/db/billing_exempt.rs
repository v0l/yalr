use super::Database;

impl Database {
    pub async fn is_billing_exempt(&self, user_id: i64) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM billing_exempt_users WHERE user_id = ?)
                 OR COALESCE((SELECT is_admin FROM users WHERE id = ?), 0)",
        )
        .bind(user_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn admins_and_listed_users_are_exempt() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        for (name, admin) in [("root", 1), ("friend", 0), ("stranger", 0)] {
            sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES (?, 0, ?)")
                .bind(name)
                .bind(admin)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO billing_exempt_users (user_id) VALUES (2)")
            .execute(&db.pool)
            .await
            .unwrap();
        assert!(db.is_billing_exempt(1).await.unwrap());
        assert!(db.is_billing_exempt(2).await.unwrap());
        assert!(!db.is_billing_exempt(3).await.unwrap());
        assert!(!db.is_billing_exempt(99).await.unwrap());
    }
}
