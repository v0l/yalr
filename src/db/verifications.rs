use super::{Database, Provider};

#[derive(Clone, Debug, sqlx::FromRow, serde::Serialize)]
pub struct VerificationRow {
    pub id: i64,
    pub run_id: String,
    pub provider_id: i64,
    pub model_name: String,
    pub reference_provider_id: Option<i64>,
    pub outcome: String,
    pub passed: bool,
    pub reason: Option<String>,
    pub scores: Option<String>,
    pub results: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct NewVerification<'a> {
    pub run_id: &'a str,
    pub provider_id: i64,
    pub model_name: &'a str,
    pub reference_provider_id: Option<i64>,
    pub outcome: &'a str,
    pub passed: bool,
    pub reason: Option<&'a str>,
    pub scores: &'a str,
    pub results: &'a str,
}

impl Database {
    pub async fn insert_verification(&self, v: NewVerification<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO provider_verifications \
             (run_id, provider_id, model_name, reference_provider_id, outcome, passed, reason, scores, results) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(v.run_id)
        .bind(v.provider_id)
        .bind(v.model_name)
        .bind(v.reference_provider_id)
        .bind(v.outcome)
        .bind(v.passed)
        .bind(v.reason)
        .bind(v.scores)
        .bind(v.results)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn latest_verifications(&self, provider_id: i64) -> Result<Vec<VerificationRow>, sqlx::Error> {
        sqlx::query_as::<_, VerificationRow>(
            "SELECT v.* FROM provider_verifications v WHERE v.provider_id = ? AND v.id = \
             (SELECT MAX(id) FROM provider_verifications w WHERE w.provider_id = v.provider_id AND w.model_name = v.model_name) \
             ORDER BY v.model_name",
        )
        .bind(provider_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn list_verifications(&self, provider_id: i64, limit: i64) -> Result<Vec<VerificationRow>, sqlx::Error> {
        sqlx::query_as::<_, VerificationRow>(
            "SELECT * FROM provider_verifications WHERE provider_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(provider_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn providers_due_for_verification(&self, interval_hours: u64) -> Result<Vec<Provider>, sqlx::Error> {
        sqlx::query_as::<_, Provider>(
            "SELECT p.* FROM providers p WHERE p.owner_user_id IS NOT NULL AND p.is_reference = FALSE \
             AND p.status = 'approved' \
             AND p.status_locked = FALSE AND EXISTS (SELECT 1 FROM model_pricing mp WHERE mp.provider_id = p.id) \
             AND COALESCE((SELECT MAX(v.created_at) FROM provider_verifications v WHERE v.provider_id = p.id), '1970-01-02') \
                 < DATE_SUB(UTC_TIMESTAMP(3), INTERVAL ? HOUR) ORDER BY p.id",
        )
        .bind(interval_hours as i64)
        .fetch_all(&self.pool)
        .await
    }
}
