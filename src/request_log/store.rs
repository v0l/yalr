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
    pub client_request_packed: Option<Vec<u8>>,
    pub upstream_request_packed: Option<Vec<u8>>,
    pub response_packed: Option<Vec<u8>>,
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
    if r.client_request_packed.is_some() || r.upstream_request_packed.is_some() || r.response_packed.is_some() {
        sqlx::query(
            "INSERT INTO request_log_body (log_id, client_request_packed, upstream_request_packed, response_packed)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&r.client_request_packed)
        .bind(&r.upstream_request_packed)
        .bind(&r.response_packed)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(id)
}

pub struct Retention {
    pub body_days: u32,
    pub metadata_days: u32,
    pub max_body_bytes: u64,
}

const DELETE_BATCH: usize = 10;

async fn delete_in_batches(pool: &SqlitePool, table: &str, column: &str, ids: Vec<i64>) -> Result<u64, sqlx::Error> {
    let mut removed = 0;
    for batch in ids.chunks(DELETE_BATCH) {
        let mut query = sqlx::QueryBuilder::new(format!("DELETE FROM {table} WHERE {column} IN ("));
        let mut separated = query.separated(", ");
        for id in batch {
            separated.push_bind(*id);
        }
        separated.push_unseparated(")");
        removed += query.build().execute(pool).await?.rows_affected();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Ok(removed)
}

pub async fn prune(pool: &SqlitePool, now_ms: i64, retention: &Retention) -> Result<u64, sqlx::Error> {
    let day_ms = 86_400_000i64;
    let mut removed = 0;
    if retention.body_days > 0 {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT log_id FROM request_log_body WHERE log_id IN
             (SELECT id FROM request_log WHERE started_at_ms < ?)",
        )
        .bind(now_ms - i64::from(retention.body_days) * day_ms)
        .fetch_all(pool)
        .await?;
        removed += delete_in_batches(pool, "request_log_body", "log_id", ids).await?;
    }
    if retention.max_body_bytes > 0 {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT log_id FROM (
                SELECT log_id,
                       SUM(size) OVER (ORDER BY log_id DESC) AS newest_first_total
                FROM (
                    SELECT log_id,
                           COALESCE(LENGTH(client_request_packed), 0)
                         + COALESCE(LENGTH(upstream_request_packed), 0)
                         + COALESCE(LENGTH(response_packed), 0) AS size
                    FROM request_log_body
                )
            ) WHERE newest_first_total > ?",
        )
        .bind(i64::try_from(retention.max_body_bytes).unwrap_or(i64::MAX))
        .fetch_all(pool)
        .await?;
        removed += delete_in_batches(pool, "request_log_body", "log_id", ids).await?;
    }
    if retention.metadata_days > 0 {
        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM request_log WHERE started_at_ms < ?")
            .bind(now_ms - i64::from(retention.metadata_days) * day_ms)
            .fetch_all(pool)
            .await?;
        removed += delete_in_batches(pool, "request_log", "id", ids).await?;
    }
    Ok(removed)
}

pub async fn enable_incremental_vacuum(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let mode: i64 = sqlx::query_scalar("PRAGMA auto_vacuum").fetch_one(&mut *connection).await?;
    if mode == 2 {
        return Ok(false);
    }
    sqlx::query("PRAGMA auto_vacuum = INCREMENTAL").execute(&mut *connection).await?;
    sqlx::query("VACUUM").execute(&mut *connection).await?;
    Ok(true)
}

const VACUUM_PAGES: i64 = 256;

pub async fn release_freed_pages(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut remaining: i64 = sqlx::query_scalar("PRAGMA freelist_count").fetch_one(pool).await?;
    while remaining > 0 {
        sqlx::query(&format!("PRAGMA incremental_vacuum({VACUUM_PAGES})")).execute(pool).await?;
        let now: i64 = sqlx::query_scalar("PRAGMA freelist_count").fetch_one(pool).await?;
        if now >= remaining {
            break;
        }
        remaining = now;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    sqlx::query("PRAGMA wal_checkpoint(PASSIVE)").execute(pool).await?;
    Ok(())
}

pub async fn body_bytes(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(COALESCE(LENGTH(client_request_packed), 0)
                          + COALESCE(LENGTH(upstream_request_packed), 0)
                          + COALESCE(LENGTH(response_packed), 0)), 0)
         FROM request_log_body",
    )
    .fetch_one(pool)
    .await
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
    pub api_key_id: Option<i64>,
    pub api_key_name: Option<String>,
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

#[derive(Default)]
pub struct Filter<'a> {
    pub before: Option<i64>,
    pub model: Option<&'a str>,
    pub status: Option<&'a str>,
    pub user_id: Option<i64>,
    pub api_key_id: Option<i64>,
    pub since_ms: Option<i64>,
}

pub async fn list(pool: &SqlitePool, limit: i64, filter: Filter<'_>) -> Result<Vec<Summary>, sqlx::Error> {
    let mut query = sqlx::QueryBuilder::new(
        "SELECT id, request_id, started_at_ms, model, stream, provider, status, error, attempts,
                user_name, api_key_id, api_key_name, client_ip, headers_ms, first_event_ms,
                first_thinking_ms, first_text_ms,
                total_ms, prompt_tokens, completion_tokens, cached_tokens, cache_write_tokens,
                weighted_input_tokens, message_count, first_diverging_message, previous_request_id
         FROM request_log WHERE 1 = 1",
    );
    if let Some(before) = filter.before {
        query.push(" AND id < ").push_bind(before);
    }
    if let Some(model) = filter.model {
        query.push(" AND model = ").push_bind(model);
    }
    if let Some(status) = filter.status {
        query.push(" AND status = ").push_bind(status);
    }
    if let Some(user_id) = filter.user_id {
        query.push(" AND user_id = ").push_bind(user_id);
    }
    if let Some(api_key_id) = filter.api_key_id {
        query.push(" AND api_key_id = ").push_bind(api_key_id);
    }
    if let Some(since_ms) = filter.since_ms {
        query.push(" AND started_at_ms >= ").push_bind(since_ms);
    }
    query.push(" ORDER BY id DESC LIMIT ").push_bind(limit);
    query.build_query_as::<Summary>().fetch_all(pool).await
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

pub async fn has_bodies(pool: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM request_log_body WHERE log_id = ?)")
        .bind(id)
        .fetch_one(pool)
        .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct Bodies {
    pub client_request_packed: Option<Vec<u8>>,
    pub upstream_request_packed: Option<Vec<u8>>,
    pub response_packed: Option<Vec<u8>>,
}

pub async fn bodies(pool: &SqlitePool, id: i64) -> Result<Option<Bodies>, sqlx::Error> {
    sqlx::query_as::<_, Bodies>(
        "SELECT client_request_packed, upstream_request_packed, response_packed FROM request_log_body WHERE log_id = ?",
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
            client_request_packed: with_bodies.then(|| vec![1, 2, 3]),
            upstream_request_packed: with_bodies.then(|| vec![4, 5]),
            response_packed: with_bodies.then(|| vec![6]),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn insert_then_list_detail_and_bodies_round_trip() {
        let pool = pool().await;
        let id = insert(&pool, record("r1", 1_000, true)).await.unwrap();
        let rows = list(&pool, 10, Filter::default()).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_id, "r1");
        assert_eq!(rows[0].cached_tokens, Some(900));
        let found = detail(&pool, id).await.unwrap().unwrap();
        assert_eq!(found.model, "code-high");
        assert!(found.stream);
        let stored = bodies(&pool, id).await.unwrap().unwrap();
        assert_eq!(stored.client_request_packed, Some(vec![1, 2, 3]));
        assert_eq!(stored.response_packed, Some(vec![6]));
    }

    #[tokio::test]
    async fn list_scopes_to_user_and_key() {
        let pool = pool().await;
        for (id, user, key) in [("a", 1, 10), ("b", 1, 11), ("c", 2, 12)] {
            let mut r = record(id, 1, false);
            r.user_id = Some(user);
            r.api_key_id = Some(key);
            insert(&pool, r).await.unwrap();
        }
        let mine = list(&pool, 10, Filter { user_id: Some(1), ..Default::default() }).await.unwrap();
        assert_eq!(mine.len(), 2);
        let one = list(&pool, 10, Filter { api_key_id: Some(11), ..Default::default() }).await.unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].request_id, "b");
        assert_eq!(one[0].api_key_id, Some(11));
        let late = list(&pool, 10, Filter { since_ms: Some(2), ..Default::default() }).await.unwrap();
        assert!(late.is_empty());
    }

    #[tokio::test]
    async fn request_id_lookup_uses_an_index() {
        let pool = pool().await;
        let rows = sqlx::query("EXPLAIN QUERY PLAN SELECT id FROM request_log WHERE request_id = 'x'")
            .fetch_all(&pool)
            .await
            .unwrap();
        use sqlx::Row;
        let detail: String = rows[0].get("detail");
        assert!(detail.contains("USING"), "{detail}");
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
        assert!(!has_bodies(&pool, id).await.unwrap());
        let with = insert(&pool, record("r2", 1_000, true)).await.unwrap();
        assert!(has_bodies(&pool, with).await.unwrap());
    }

    #[tokio::test]
    async fn list_filters_and_pages_newest_first() {
        let pool = pool().await;
        let first = insert(&pool, record("a", 1, false)).await.unwrap();
        let mut failed = record("b", 2, false);
        failed.status = "error".into();
        let second = insert(&pool, failed).await.unwrap();
        let all = list(&pool, 10, Filter::default()).await.unwrap();
        assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), vec![second, first]);
        let errors = list(&pool, 10, Filter { status: Some("error"), ..Default::default() }).await.unwrap();
        assert_eq!(errors.len(), 1);
        let older = list(&pool, 10, Filter { before: Some(second), ..Default::default() }).await.unwrap();
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
        prune(&pool, now, &Retention { body_days: 7, metadata_days: 0, max_body_bytes: 0 }).await.unwrap();
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
        prune(&pool, now, &Retention { body_days: 7, metadata_days: 30, max_body_bytes: 0 }).await.unwrap();
        assert!(detail(&pool, old).await.unwrap().is_none());
        assert!(bodies(&pool, old).await.unwrap().is_none());
    }

    fn sized(request_id: &str, started_at_ms: i64, bytes: usize) -> Record {
        Record {
            client_request_packed: Some(vec![7; bytes]),
            upstream_request_packed: Some(vec![8; bytes]),
            response_packed: None,
            ..record(request_id, started_at_ms, false)
        }
    }

    #[tokio::test]
    async fn prune_keeps_the_newest_bodies_within_the_byte_cap() {
        let pool = pool().await;
        let now = 100 * 86_400_000;
        let mut ids = Vec::new();
        for n in 0..6 {
            ids.push(insert(&pool, sized(&format!("r{n}"), now - (6 - n) * 1000, 1000)).await.unwrap());
        }
        assert_eq!(body_bytes(&pool).await.unwrap(), 12_000);
        let keep = Retention { body_days: 7, metadata_days: 0, max_body_bytes: 5_000 };
        prune(&pool, now, &keep).await.unwrap();
        assert_eq!(body_bytes(&pool).await.unwrap(), 4_000);
        for id in &ids[..4] {
            assert!(bodies(&pool, *id).await.unwrap().is_none());
            assert!(detail(&pool, *id).await.unwrap().is_some());
        }
        for id in &ids[4..] {
            assert!(bodies(&pool, *id).await.unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn a_zero_byte_cap_means_unlimited() {
        let pool = pool().await;
        let now = 100 * 86_400_000;
        for n in 0..3 {
            insert(&pool, sized(&format!("r{n}"), now - 1000, 1000)).await.unwrap();
        }
        prune(&pool, now, &Retention { body_days: 7, metadata_days: 0, max_body_bytes: 0 }).await.unwrap();
        assert_eq!(body_bytes(&pool).await.unwrap(), 6_000);
    }

    #[tokio::test]
    async fn pruned_space_is_returned_to_the_file() {
        let dir = std::env::temp_dir().join(format!("yalr-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.db");
        let pool = Database::new(&format!("sqlite://{}?mode=rwc", file.display())).await.unwrap().pool;
        let now = 100 * 86_400_000;
        for n in 0..40 {
            insert(&pool, sized(&format!("r{n}"), now - 1000 + n, 100_000)).await.unwrap();
        }
        assert!(enable_incremental_vacuum(&pool).await.unwrap());
        pool.close().await;
        let pool = Database::new(&format!("sqlite://{}?mode=rwc", file.display())).await.unwrap().pool;
        assert!(!enable_incremental_vacuum(&pool).await.unwrap());
        let full = std::fs::metadata(&file).unwrap().len();
        assert!(full > 7_000_000);
        let keep = Retention { body_days: 7, metadata_days: 0, max_body_bytes: 1_000_000 };
        prune(&pool, now, &keep).await.unwrap();
        release_freed_pages(&pool).await.unwrap();
        let trimmed = std::fs::metadata(&file).unwrap().len();
        assert!(trimmed < full / 4, "{trimmed} vs {full}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
