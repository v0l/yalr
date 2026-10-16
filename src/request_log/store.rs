use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Default)]
pub struct Record {
    pub request_id: String,
    pub started_at_ms: i64,
    pub endpoint: String,
    pub stream: bool,
    pub model: String,
    pub user_id: Option<i64>,
    pub user_name: Option<String>,
    pub api_key_id: Option<i64>,
    pub api_key_name: Option<String>,
    pub client_ip: Option<String>,
    pub client_headers: String,
    pub provider: Option<String>,
    pub upstream_url: Option<String>,
    pub upstream_request_headers: String,
    pub upstream_status: Option<i64>,
    pub upstream_response_headers: String,
    pub attempts: i64,
    pub attempt_log: String,
    pub status: String,
    pub error: Option<String>,
    pub finish_reason: Option<String>,
    pub headers_ms: Option<i64>,
    pub first_event_ms: Option<i64>,
    pub first_thinking_ms: Option<i64>,
    pub first_text_ms: Option<i64>,
    pub total_ms: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub weighted_input_tokens: Option<i64>,
    pub message_count: Option<i64>,
    pub system_hash: Option<String>,
    pub tools_hash: Option<String>,
    pub message_hashes: String,
    pub first_diverging_message: Option<i64>,
    pub previous_request_id: Option<String>,
    pub request_bytes: i64,
    pub response_bytes: i64,
    pub client_request_gz: Option<Vec<u8>>,
    pub upstream_request_gz: Option<Vec<u8>>,
    pub response_gz: Option<Vec<u8>>,
}

pub async fn insert(pool: &SqlitePool, r: Record) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let id = sqlx::query(
        "INSERT INTO request_log (
            request_id, started_at_ms, endpoint, stream, model, user_id, user_name,
            api_key_id, api_key_name, client_ip, client_headers, provider, upstream_url,
            upstream_request_headers, upstream_status, upstream_response_headers, attempts,
            attempt_log, status, error, finish_reason, headers_ms, first_event_ms,
            first_thinking_ms, first_text_ms, total_ms, prompt_tokens, completion_tokens,
            cached_tokens, cache_write_tokens, weighted_input_tokens, message_count,
            system_hash, tools_hash, message_hashes, first_diverging_message,
            previous_request_id, request_bytes, response_bytes
        ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&r.request_id)
    .bind(r.started_at_ms)
    .bind(&r.endpoint)
    .bind(r.stream)
    .bind(&r.model)
    .bind(r.user_id)
    .bind(&r.user_name)
    .bind(r.api_key_id)
    .bind(&r.api_key_name)
    .bind(&r.client_ip)
    .bind(&r.client_headers)
    .bind(&r.provider)
    .bind(&r.upstream_url)
    .bind(&r.upstream_request_headers)
    .bind(r.upstream_status)
    .bind(&r.upstream_response_headers)
    .bind(r.attempts)
    .bind(&r.attempt_log)
    .bind(&r.status)
    .bind(&r.error)
    .bind(&r.finish_reason)
    .bind(r.headers_ms)
    .bind(r.first_event_ms)
    .bind(r.first_thinking_ms)
    .bind(r.first_text_ms)
    .bind(r.total_ms)
    .bind(r.prompt_tokens)
    .bind(r.completion_tokens)
    .bind(r.cached_tokens)
    .bind(r.cache_write_tokens)
    .bind(r.weighted_input_tokens)
    .bind(r.message_count)
    .bind(&r.system_hash)
    .bind(&r.tools_hash)
    .bind(&r.message_hashes)
    .bind(r.first_diverging_message)
    .bind(&r.previous_request_id)
    .bind(r.request_bytes)
    .bind(r.response_bytes)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    if r.client_request_gz.is_some() || r.upstream_request_gz.is_some() || r.response_gz.is_some() {
        sqlx::query(
            "INSERT INTO request_log_body (log_id, client_request_gz, upstream_request_gz, response_gz)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&r.client_request_gz)
        .bind(&r.upstream_request_gz)
        .bind(&r.response_gz)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

pub async fn prune(pool: &SqlitePool, now_ms: i64, body_days: u32, metadata_days: u32) -> Result<u64, sqlx::Error> {
    let day_ms = 86_400_000i64;
    let mut removed = 0;
    if body_days > 0 {
        removed += sqlx::query(
            "DELETE FROM request_log_body WHERE log_id IN
             (SELECT id FROM request_log WHERE started_at_ms < ?)",
        )
        .bind(now_ms - i64::from(body_days) * day_ms)
        .execute(pool)
        .await?
        .rows_affected();
    }
    if metadata_days > 0 {
        removed += sqlx::query("DELETE FROM request_log WHERE started_at_ms < ?")
            .bind(now_ms - i64::from(metadata_days) * day_ms)
            .execute(pool)
            .await?
            .rows_affected();
    }
    Ok(removed)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Summary {
    pub id: i64,
    pub request_id: String,
    pub started_at_ms: i64,
    pub model: String,
    pub stream: bool,
    pub provider: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub attempts: i64,
    pub user_name: Option<String>,
    pub client_ip: Option<String>,
    pub headers_ms: Option<i64>,
    pub first_event_ms: Option<i64>,
    pub first_thinking_ms: Option<i64>,
    pub first_text_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub weighted_input_tokens: Option<i64>,
    pub message_count: Option<i64>,
    pub first_diverging_message: Option<i64>,
    pub previous_request_id: Option<String>,
}

pub async fn list(
    pool: &SqlitePool,
    limit: i64,
    before: Option<i64>,
    model: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Summary>, sqlx::Error> {
    sqlx::query_as::<_, Summary>(
        "SELECT id, request_id, started_at_ms, model, stream, provider, status, error, attempts,
                user_name, client_ip, headers_ms, first_event_ms, first_thinking_ms, first_text_ms,
                total_ms, prompt_tokens, completion_tokens, cached_tokens, cache_write_tokens,
                weighted_input_tokens, message_count, first_diverging_message, previous_request_id
         FROM request_log
         WHERE (?1 IS NULL OR id < ?1) AND (?2 IS NULL OR model = ?2) AND (?3 IS NULL OR status = ?3)
         ORDER BY id DESC LIMIT ?4",
    )
    .bind(before)
    .bind(model)
    .bind(status)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Detail {
    pub id: i64,
    pub request_id: String,
    pub started_at_ms: i64,
    pub endpoint: String,
    pub stream: bool,
    pub model: String,
    pub user_id: Option<i64>,
    pub user_name: Option<String>,
    pub api_key_id: Option<i64>,
    pub api_key_name: Option<String>,
    pub client_ip: Option<String>,
    pub client_headers: Option<String>,
    pub provider: Option<String>,
    pub upstream_url: Option<String>,
    pub upstream_request_headers: Option<String>,
    pub upstream_status: Option<i64>,
    pub upstream_response_headers: Option<String>,
    pub attempts: i64,
    pub attempt_log: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub finish_reason: Option<String>,
    pub headers_ms: Option<i64>,
    pub first_event_ms: Option<i64>,
    pub first_thinking_ms: Option<i64>,
    pub first_text_ms: Option<i64>,
    pub total_ms: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub weighted_input_tokens: Option<i64>,
    pub message_count: Option<i64>,
    pub system_hash: Option<String>,
    pub tools_hash: Option<String>,
    pub message_hashes: Option<String>,
    pub first_diverging_message: Option<i64>,
    pub previous_request_id: Option<String>,
    pub request_bytes: Option<i64>,
    pub response_bytes: Option<i64>,
}

pub async fn detail(pool: &SqlitePool, id: i64) -> Result<Option<Detail>, sqlx::Error> {
    sqlx::query_as::<_, Detail>("SELECT * FROM request_log WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn id_for_request(pool: &SqlitePool, request_id: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM request_log WHERE request_id = ?")
        .bind(request_id)
        .fetch_optional(pool)
        .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct Bodies {
    pub client_request_gz: Option<Vec<u8>>,
    pub upstream_request_gz: Option<Vec<u8>>,
    pub response_gz: Option<Vec<u8>>,
}

pub async fn bodies(pool: &SqlitePool, id: i64) -> Result<Option<Bodies>, sqlx::Error> {
    sqlx::query_as::<_, Bodies>(
        "SELECT client_request_gz, upstream_request_gz, response_gz FROM request_log_body WHERE log_id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn pool() -> SqlitePool {
        Database::new("sqlite::memory:").await.unwrap().pool
    }

    fn record(request_id: &str, started_at_ms: i64, with_bodies: bool) -> Record {
        Record {
            request_id: request_id.to_string(),
            started_at_ms,
            endpoint: "/v1/chat/completions".into(),
            stream: true,
            model: "code-high".into(),
            status: "ok".into(),
            attempts: 1,
            total_ms: 1200,
            prompt_tokens: Some(1000),
            cached_tokens: Some(900),
            client_request_gz: with_bodies.then(|| vec![1, 2, 3]),
            upstream_request_gz: with_bodies.then(|| vec![4, 5]),
            response_gz: with_bodies.then(|| vec![6]),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn insert_then_list_detail_and_bodies_round_trip() {
        let pool = pool().await;
        let id = insert(&pool, record("r1", 1_000, true)).await.unwrap();
        let rows = list(&pool, 10, None, None, None).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "r1");
        assert_eq!(rows[0].cached_tokens, Some(900));
        let found = detail(&pool, id).await.unwrap().unwrap();
        assert_eq!(found.model, "code-high");
        assert!(found.stream);
        let stored = bodies(&pool, id).await.unwrap().unwrap();
        assert_eq!(stored.client_request_gz, Some(vec![1, 2, 3]));
        assert_eq!(stored.response_gz, Some(vec![6]));
    }

    #[tokio::test]
    async fn request_id_resolves_to_row_id() {
        let pool = pool().await;
        let id = insert(&pool, record("r1", 1_000, false)).await.unwrap();
        assert_eq!(id_for_request(&pool, "r1").await.unwrap(), Some(id));
        assert_eq!(id_for_request(&pool, "missing").await.unwrap(), None);
    }

    #[tokio::test]
    async fn rows_without_bodies_have_no_body_row() {
        let pool = pool().await;
        let id = insert(&pool, record("r1", 1_000, false)).await.unwrap();
        assert!(bodies(&pool, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_filters_and_pages_newest_first() {
        let pool = pool().await;
        let first = insert(&pool, record("a", 1, false)).await.unwrap();
        let mut failed = record("b", 2, false);
        failed.status = "error".into();
        let second = insert(&pool, failed).await.unwrap();
        let all = list(&pool, 10, None, None, None).await.unwrap();
        assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), vec![second, first]);
        let errors = list(&pool, 10, None, None, Some("error")).await.unwrap();
        assert_eq!(errors.len(), 1);
        let older = list(&pool, 10, Some(second), None, None).await.unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].id, first);
    }

    #[tokio::test]
    async fn prune_drops_old_bodies_but_keeps_metadata() {
        let pool = pool().await;
        let day = 86_400_000;
        let now = 100 * day;
        let old = insert(&pool, record("old", now - 8 * day, true)).await.unwrap();
        let fresh = insert(&pool, record("fresh", now - day, true)).await.unwrap();
        prune(&pool, now, 7, 0).await.unwrap();
        assert!(bodies(&pool, old).await.unwrap().is_none());
        assert!(bodies(&pool, fresh).await.unwrap().is_some());
        assert!(detail(&pool, old).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn prune_can_also_expire_metadata() {
        let pool = pool().await;
        let day = 86_400_000;
        let now = 100 * day;
        let old = insert(&pool, record("old", now - 40 * day, true)).await.unwrap();
        prune(&pool, now, 7, 30).await.unwrap();
        assert!(detail(&pool, old).await.unwrap().is_none());
        assert!(bodies(&pool, old).await.unwrap().is_none());
    }
}
