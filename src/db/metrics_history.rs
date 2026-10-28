use super::Database;

impl Database {
    pub async fn recent_metrics_snapshots(&self, limit: i64) -> Result<Vec<(i64, String)>, sqlx::Error> {
        sqlx::query_as("SELECT timestamp_ms, snapshot_json FROM metrics_history ORDER BY timestamp_ms DESC LIMIT ?")
            .bind(limit)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn save_metrics_snapshot(&self, timestamp_ms: i64, json: &str) -> Result<(), sqlx::Error> {
        sqlx::query("REPLACE INTO metrics_history (timestamp_ms, snapshot_json) VALUES (?, ?)")
            .bind(timestamp_ms)
            .bind(json)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_metrics_snapshots(&self, older_than_ms: i64, keep: i64) -> Result<u64, sqlx::Error> {
        let oldest_kept: Option<i64> = sqlx::query_scalar(
            "SELECT timestamp_ms FROM metrics_history ORDER BY timestamp_ms DESC LIMIT 1 OFFSET ?",
        )
        .bind(keep.max(1) - 1)
        .fetch_optional(&self.pool)
        .await?;
        let cutoff = oldest_kept.map_or(older_than_ms, |t| t.max(older_than_ms));
        let result = sqlx::query("DELETE FROM metrics_history WHERE timestamp_ms < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn prune_keeps_the_newest_snapshots_inside_the_window() {
        let db = Database::test().await;
        for t in 1..=5 {
            db.save_metrics_snapshot(t * 1000, "{}").await.unwrap();
        }
        db.save_metrics_snapshot(5000, "{\"replaced\":true}").await.unwrap();
        assert_eq!(db.prune_metrics_snapshots(2000, 3).await.unwrap(), 2);
        let left: Vec<i64> = db.recent_metrics_snapshots(10).await.unwrap().into_iter().map(|r| r.0).collect();
        assert_eq!(left, vec![5000, 4000, 3000]);
        assert_eq!(db.recent_metrics_snapshots(1).await.unwrap()[0].1, "{\"replaced\":true}");
    }
}
