use super::Database;

#[derive(Debug, Clone, Default, sqlx::FromRow, PartialEq)]
pub struct HealthUptime {
    pub provider_id: i64,
    pub checks_24h: i64,
    pub healthy_24h: i64,
    pub checks_7d: i64,
    pub healthy_7d: i64,
    pub last_healthy_ms: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RequestCount {
    pub model: String,
    pub upstream_model: Option<String>,
    pub provider: Option<String>,
    pub requests: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RequestSample {
    pub model: String,
    pub upstream_model: Option<String>,
    pub provider: Option<String>,
    pub status: String,
    pub started_at_ms: i64,
    pub first_event_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub completion_tokens: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SpeedRollup {
    pub provider: String,
    pub model: String,
    pub tokens_per_second: Option<f64>,
    pub ttft_ms: Option<f64>,
}

impl Database {
    pub async fn speed_rollup(&self, since_ms: i64, min_samples: i64) -> Result<Vec<SpeedRollup>, sqlx::Error> {
        sqlx::query_as(
            "SELECT provider, model,
                    CAST(SUM(completion_tokens) * 1000 / NULLIF(SUM(total_ms - COALESCE(first_event_ms, 0)), 0) AS DOUBLE) AS tokens_per_second,
                    CAST(AVG(first_event_ms) AS DOUBLE) AS ttft_ms
             FROM request_log
             WHERE started_at_ms >= ? AND status = 'ok' AND provider IS NOT NULL
               AND completion_tokens > 0 AND total_ms > COALESCE(first_event_ms, 0)
             GROUP BY provider, model HAVING COUNT(*) >= ?",
        )
        .bind(since_ms)
        .bind(min_samples)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn record_health_check(&self, provider_id: i64, healthy: bool, at_ms: i64) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO provider_health_checks (provider_id, checked_at_ms, healthy) VALUES (?, ?, ?)")
            .bind(provider_id)
            .bind(at_ms)
            .bind(healthy)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_health_checks(&self, before_ms: i64) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query("DELETE FROM provider_health_checks WHERE checked_at_ms < ?")
            .bind(before_ms)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }

    pub async fn health_uptime(&self, since_24h_ms: i64, since_7d_ms: i64) -> Result<Vec<HealthUptime>, sqlx::Error> {
        sqlx::query_as(
            "SELECT provider_id,
                CAST(COALESCE(SUM(checked_at_ms >= ?), 0) AS SIGNED) AS checks_24h,
                CAST(COALESCE(SUM(healthy AND checked_at_ms >= ?), 0) AS SIGNED) AS healthy_24h,
                CAST(COUNT(*) AS SIGNED) AS checks_7d,
                CAST(COALESCE(SUM(healthy), 0) AS SIGNED) AS healthy_7d,
                MAX(CASE WHEN healthy THEN checked_at_ms END) AS last_healthy_ms
             FROM provider_health_checks WHERE checked_at_ms >= ? GROUP BY provider_id",
        )
        .bind(since_24h_ms)
        .bind(since_24h_ms)
        .bind(since_7d_ms)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn request_counts_since(&self, since_ms: i64) -> Result<Vec<RequestCount>, sqlx::Error> {
        sqlx::query_as(
            "SELECT model, upstream_model, provider, CAST(COUNT(*) AS SIGNED) AS requests
             FROM request_log WHERE started_at_ms >= ? GROUP BY model, upstream_model, provider",
        )
        .bind(since_ms)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn request_samples_since(&self, since_ms: i64, limit: i64) -> Result<Vec<RequestSample>, sqlx::Error> {
        sqlx::query_as(
            "SELECT model, upstream_model, provider, status, started_at_ms, first_event_ms, total_ms, completion_tokens
             FROM request_log WHERE started_at_ms >= ?
             ORDER BY started_at_ms DESC LIMIT ?",
        )
        .bind(since_ms)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uptime_counts_checks_per_window_and_prune_drops_old_ones() {
        let db = Database::test().await;
        let p = db
            .create_provider(crate::db::NewProvider { name: "p", slug: "p", base_url: "http://x", api_key: None, provider_type: None })
            .await
            .unwrap();
        let hour = 3_600_000;
        let now = 10 * 24 * hour;
        for (age_h, healthy) in [(1, true), (2, false), (3, true), (30, true), (50, false), (9 * 24, true)] {
            db.record_health_check(p.id, healthy, now - age_h * hour).await.unwrap();
        }
        let rows = db.health_uptime(now - 24 * hour, now - 7 * 24 * hour).await.unwrap();
        assert_eq!(
            rows,
            vec![HealthUptime {
                provider_id: p.id,
                checks_24h: 3,
                healthy_24h: 2,
                checks_7d: 5,
                healthy_7d: 3,
                last_healthy_ms: Some(now - hour),
            }]
        );
        assert_eq!(db.prune_health_checks(now - 8 * 24 * hour).await.unwrap(), 1);
    }
}
