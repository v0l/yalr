use super::Database;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow, PartialEq)]
pub struct ProviderTraffic {
    pub requests: i64,
    pub failures: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub avg_ttft_ms: Option<f64>,
    pub output_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, PartialEq)]
pub struct ProviderEarning {
    pub currency: String,
    pub earned_milli: i64,
    pub requests: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, PartialEq)]
pub struct ProviderModelTraffic {
    pub model: String,
    pub requests: i64,
    pub completion_tokens: i64,
    pub output_tokens_per_second: Option<f64>,
}

const TRAFFIC_SUMS: &str = "CAST(COUNT(*) AS SIGNED) AS requests,
    CAST(COALESCE(SUM(status <> 'ok'), 0) AS SIGNED) AS failures,
    CAST(COALESCE(SUM(prompt_tokens), 0) AS SIGNED) AS prompt_tokens,
    CAST(COALESCE(SUM(completion_tokens), 0) AS SIGNED) AS completion_tokens,
    CAST(COALESCE(SUM(cached_tokens), 0) AS SIGNED) AS cached_tokens";

const OUTPUT_TPS: &str = "CAST(SUM(CASE WHEN completion_tokens > 0 AND total_ms > COALESCE(first_event_ms, 0)
        THEN completion_tokens END) * 1000.0 /
    NULLIF(SUM(CASE WHEN completion_tokens > 0 AND total_ms > COALESCE(first_event_ms, 0)
        THEN total_ms - COALESCE(first_event_ms, 0) END), 0) AS DOUBLE)";

impl Database {
    pub async fn provider_traffic(&self, provider_name: &str, since_ms: i64) -> Result<ProviderTraffic, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {TRAFFIC_SUMS},
                CAST(AVG(first_event_ms) AS DOUBLE) AS avg_ttft_ms,
                {OUTPUT_TPS} AS output_tokens_per_second
             FROM request_log WHERE provider = ? AND started_at_ms >= ?"
        ))
        .bind(provider_name)
        .bind(since_ms)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn provider_model_traffic(
        &self,
        provider_name: &str,
        since_ms: i64,
    ) -> Result<Vec<ProviderModelTraffic>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT model,
                CAST(COUNT(*) AS SIGNED) AS requests,
                CAST(COALESCE(SUM(completion_tokens), 0) AS SIGNED) AS completion_tokens,
                {OUTPUT_TPS} AS output_tokens_per_second
             FROM request_log WHERE provider = ? AND started_at_ms >= ?
             GROUP BY model ORDER BY requests DESC, model"
        ))
        .bind(provider_name)
        .bind(since_ms)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn provider_earnings(
        &self,
        owner_user_id: i64,
        provider_name: &str,
        since_ms: i64,
    ) -> Result<Vec<ProviderEarning>, sqlx::Error> {
        sqlx::query_as(
            "SELECT bt.currency AS currency,
                CAST(SUM(CASE WHEN bt.currency = 'sats' THEN bt.amount_msat ELSE bt.amount_milli END) AS SIGNED)
                    AS earned_milli,
                CAST(COUNT(*) AS SIGNED) AS requests
             FROM balance_transactions bt
             JOIN request_log rl ON rl.request_id = bt.reference_id
             WHERE bt.user_id = ? AND bt.transaction_type = 'provider_earning'
               AND rl.provider = ? AND rl.started_at_ms >= ?
             GROUP BY bt.currency ORDER BY bt.currency",
        )
        .bind(owner_user_id)
        .bind(provider_name)
        .bind(since_ms)
        .fetch_all(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Record, Settlement};

    fn record(request_id: &str, provider: &str, model: &str, status: &str) -> Record {
        Record {
            request_id: request_id.into(),
            started_at_ms: 10_000,
            endpoint: "/v1/chat/completions".into(),
            model: model.into(),
            provider: Some(provider.into()),
            status: status.into(),
            attempts: 1,
            first_event_ms: Some(500),
            total_ms: 2_500,
            prompt_tokens: Some(100),
            completion_tokens: Some(200),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn traffic_and_earnings_are_scoped_to_one_provider() {
        let db = Database::test().await;
        let owner = db.add_test_user("owner", false).await;
        let consumer = db.add_test_user("consumer", false).await;
        db.credit_currency(consumer.id, "sats", 1_000_000, "deposit", "seed", 1_000_000).await.unwrap();
        db.insert_request_logs(&[
            record("r1", "owner/mine", "m1", "ok"),
            record("r2", "owner/mine", "m2", "error"),
            record("r3", "owner/other", "m1", "ok"),
        ])
        .await
        .unwrap();
        for id in ["r1", "r3"] {
            db.settle_usage(Settlement {
                consumer_id: consumer.id,
                owner_id: Some(owner.id),
                currency: "sats",
                amount: 10_000,
                fee_ppm: 100_000,
                reference_id: id,
                model: Some("m1"),
            })
            .await
            .unwrap();
        }

        let traffic = db.provider_traffic("owner/mine", 0).await.unwrap();
        assert_eq!((traffic.requests, traffic.failures), (2, 1));
        assert_eq!(traffic.completion_tokens, 400);
        assert_eq!(traffic.avg_ttft_ms, Some(500.0));
        assert_eq!(traffic.output_tokens_per_second, Some(100.0));

        let models = db.provider_model_traffic("owner/mine", 0).await.unwrap();
        assert_eq!(models.len(), 2);

        let earnings = db.provider_earnings(owner.id, "owner/mine", 0).await.unwrap();
        assert_eq!(earnings, vec![ProviderEarning { currency: "sats".into(), earned_milli: 9_000, requests: 1 }]);

        let later = db.provider_traffic("owner/mine", 20_000).await.unwrap();
        assert_eq!(later.requests, 0);
        assert_eq!(later.output_tokens_per_second, None);
    }
}
