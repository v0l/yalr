use super::Database;
use crate::payments::biller::TokenUsage;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow, PartialEq, Eq)]
pub struct ApiKeyUsage {
    pub requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_msat: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ModelUsage {
    pub model: String,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub usage: ApiKeyUsage,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct HourUsage {
    pub hour: String,
    #[sqlx(flatten)]
    #[serde(flatten)]
    pub usage: ApiKeyUsage,
}

#[derive(sqlx::FromRow)]
struct KeyUsage {
    api_key_id: i64,
    #[sqlx(flatten)]
    usage: ApiKeyUsage,
}

const SUMS: &str = "SUM(requests) AS requests, SUM(prompt_tokens) AS prompt_tokens,
    SUM(completion_tokens) AS completion_tokens, SUM(cached_tokens) AS cached_tokens,
    SUM(cache_write_tokens) AS cache_write_tokens, SUM(cost_msat) AS cost_msat";

impl Database {
    pub async fn record_api_key_usage(
        &self,
        api_key_id: i64,
        model: &str,
        usage: TokenUsage,
        cost_msat: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO api_key_usage
                (api_key_id, hour, model, requests, prompt_tokens, completion_tokens,
                 cached_tokens, cache_write_tokens, cost_msat)
             VALUES (?, strftime('%Y-%m-%dT%H:00:00Z', 'now'), ?, 1, ?, ?, ?, ?, ?)
             ON CONFLICT (api_key_id, hour, model) DO UPDATE SET
                requests = requests + 1,
                prompt_tokens = prompt_tokens + excluded.prompt_tokens,
                completion_tokens = completion_tokens + excluded.completion_tokens,
                cached_tokens = cached_tokens + excluded.cached_tokens,
                cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens,
                cost_msat = cost_msat + excluded.cost_msat",
        )
        .bind(api_key_id)
        .bind(model)
        .bind(usage.prompt_tokens as i64)
        .bind(usage.completion_tokens as i64)
        .bind(usage.cached_tokens as i64)
        .bind(usage.cache_write_tokens as i64)
        .bind(cost_msat)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn api_key_usage_for_user(
        &self,
        user_id: i64,
    ) -> Result<HashMap<i64, ApiKeyUsage>, sqlx::Error> {
        let rows = sqlx::query_as::<_, KeyUsage>(&format!(
            "SELECT api_key_id, {SUMS} FROM api_key_usage
             WHERE api_key_id IN (SELECT id FROM api_keys WHERE user_id = ?)
             GROUP BY api_key_id"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| (r.api_key_id, r.usage)).collect())
    }

    pub async fn usage_report(&self, scope: UsageScope, hours: u32) -> Result<UsageReport, sqlx::Error> {
        let (filter, id) = scope.filter();
        let since = since_clause(hours);
        let total = sqlx::query_as::<_, ApiKeyUsage>(&format!(
            "SELECT COALESCE(SUM(requests), 0) AS requests,
                COALESCE(SUM(prompt_tokens), 0) AS prompt_tokens,
                COALESCE(SUM(completion_tokens), 0) AS completion_tokens,
                COALESCE(SUM(cached_tokens), 0) AS cached_tokens,
                COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens,
                COALESCE(SUM(cost_msat), 0) AS cost_msat
             FROM api_key_usage WHERE {filter} AND {since}"
        ))
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let by_model = sqlx::query_as::<_, ModelUsage>(&format!(
            "SELECT model, {SUMS} FROM api_key_usage
             WHERE {filter} AND {since} GROUP BY model ORDER BY cost_msat DESC, model"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let by_hour = sqlx::query_as::<_, HourUsage>(&format!(
            "SELECT hour, {SUMS} FROM api_key_usage
             WHERE {filter} AND {since} GROUP BY hour ORDER BY hour"
        ))
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        Ok(UsageReport { hours, total, by_model, by_hour })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum UsageScope {
    Key(i64),
    User(i64),
}

impl UsageScope {
    fn filter(self) -> (&'static str, i64) {
        match self {
            UsageScope::Key(id) => ("api_key_id = ?", id),
            UsageScope::User(id) => ("api_key_id IN (SELECT id FROM api_keys WHERE user_id = ?)", id),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UsageReport {
    pub hours: u32,
    pub total: ApiKeyUsage,
    pub by_model: Vec<ModelUsage>,
    pub by_hour: Vec<HourUsage>,
}

fn since_clause(hours: u32) -> String {
    format!("hour > strftime('%Y-%m-%dT%H:00:00Z', 'now', '-{hours} hours')")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn usage_accumulates_per_key_and_model() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES ('u', 0, 0)")
            .execute(&db.pool)
            .await
            .unwrap();
        for (hash, name) in [("h1", "one"), ("h2", "two")] {
            sqlx::query("INSERT INTO api_keys (key_hash, name, user_id, last_four) VALUES (?, ?, 1, 'abcd')")
                .bind(hash)
                .bind(name)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        let usage = TokenUsage { prompt_tokens: 100, completion_tokens: 20, cached_tokens: 40, cache_write_tokens: 10 };
        db.record_api_key_usage(1, "gpt", usage, 500).await.unwrap();
        db.record_api_key_usage(1, "gpt", usage, 500).await.unwrap();
        db.record_api_key_usage(1, "other", TokenUsage::plain(5, 5), 7).await.unwrap();
        db.record_api_key_usage(2, "gpt", TokenUsage::plain(1, 1), 1).await.unwrap();

        let by_key = db.api_key_usage_for_user(1).await.unwrap();
        assert_eq!(
            by_key[&1],
            ApiKeyUsage {
                requests: 3,
                prompt_tokens: 205,
                completion_tokens: 45,
                cached_tokens: 80,
                cache_write_tokens: 20,
                cost_msat: 1007,
            }
        );
        assert_eq!(by_key[&2].requests, 1);

        let report = db.usage_report(UsageScope::Key(1), 24).await.unwrap();
        assert_eq!(report.total.requests, 3);
        assert_eq!(report.by_model.len(), 2);
        assert_eq!(report.by_model[0].model, "gpt");
        assert_eq!(report.by_hour.len(), 1);
        assert!(report.by_hour[0].hour.ends_with(":00:00Z"));

        let user = db.usage_report(UsageScope::User(1), 24).await.unwrap();
        assert_eq!(user.total.requests, 4);

        assert_eq!(db.usage_report(UsageScope::Key(3), 24).await.unwrap().total, ApiKeyUsage::default());

        sqlx::query("UPDATE api_key_usage SET hour = strftime('%Y-%m-%dT%H:00:00Z', 'now', '-30 hours') WHERE api_key_id = 2")
            .execute(&db.pool)
            .await
            .unwrap();
        assert_eq!(db.usage_report(UsageScope::Key(2), 24).await.unwrap().total.requests, 0);
        assert_eq!(db.usage_report(UsageScope::Key(2), 48).await.unwrap().total.requests, 1);
    }
}
