use super::{Database, Provider};

pub const PENDING: &str = "pending";
pub const APPROVED: &str = "approved";
pub const SUSPENDED: &str = "suspended";

impl Database {
    pub async fn set_provider_status(
        &self,
        id: i64,
        status: &str,
        reason: Option<&str>,
        locked: bool,
    ) -> Result<Provider, sqlx::Error> {
        sqlx::query(
            "UPDATE providers SET status = ?, status_reason = ?, status_locked = ?, \
             updated_at = CURRENT_TIMESTAMP(3) WHERE id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(locked)
        .bind(id)
        .execute(&self.pool)
        .await?;
        self.get_provider_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn set_provider_reference(&self, id: i64, is_reference: bool) -> Result<Provider, sqlx::Error> {
        sqlx::query("UPDATE providers SET is_reference = ? WHERE id = ?")
            .bind(is_reference)
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_provider_by_id(id).await?.ok_or(sqlx::Error::RowNotFound)
    }

    pub async fn reference_providers_for(&self, model: &str, exclude: i64) -> Result<Vec<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>(
            "SELECT p.* FROM providers p WHERE p.is_reference = TRUE AND p.status = 'approved' AND p.id <> ? \
             AND (p.owner_user_id IS NULL OR EXISTS (SELECT 1 FROM model_pricing mp WHERE mp.provider_id = p.id \
             AND mp.model_name = ?)) ORDER BY p.id",
        )
        .bind(exclude)
        .bind(model)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn priced_models_for_provider(&self, provider_id: i64) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT model_name FROM model_pricing WHERE provider_id = ? ORDER BY model_name")
            .bind(provider_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn list_providers_in_status(&self, status: &str) -> Result<Vec<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>("SELECT * FROM providers WHERE status = ? ORDER BY id")
            .bind(status)
            .fetch_all(&self.pool)
            .await
    }
}
