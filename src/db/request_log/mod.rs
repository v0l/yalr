mod types;

pub use types::{Bodies, Detail, Filter, Record, Retention, Summary};

use super::Database;
use sqlx::{MySql, QueryBuilder, Transaction};

const DELETE_BATCH: usize = 500;
const DAY_MS: i64 = 86_400_000;

const SUMMARY_COLUMNS: &str = "id, request_id, started_at_ms, model, stream, provider, status, error, attempts, \
    user_name, api_key_id, api_key_name, client_ip, headers_ms, first_event_ms, first_thinking_ms, first_text_ms, \
    total_ms, prompt_tokens, completion_tokens, cached_tokens, cache_write_tokens, weighted_input_tokens, \
    message_count, first_diverging_message, previous_request_id";

const PACKET_HEADROOM: usize = 64 * 1024;

fn body_size(r: &Record) -> usize {
    [&r.client_request_packed, &r.upstream_request_packed, &r.response_packed]
        .iter()
        .map(|b| b.as_ref().map_or(0, Vec::len))
        .sum()
}

async fn insert_one(tx: &mut Transaction<'_, MySql>, r: &Record, max_packet: usize) -> Result<i64, sqlx::Error> {
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
    .execute(&mut **tx)
    .await?
    .last_insert_id() as i64;
    let size = body_size(r);
    if size + PACKET_HEADROOM > max_packet {
        tracing::warn!(request_id = %r.request_id, bytes = size, "Request log body exceeds max_allowed_packet, not stored");
    } else if r.client_request_packed.is_some() || r.upstream_request_packed.is_some() || r.response_packed.is_some() {
        sqlx::query(
            "INSERT INTO request_log_body (log_id, client_request_packed, upstream_request_packed, response_packed) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&r.client_request_packed)
        .bind(&r.upstream_request_packed)
        .bind(&r.response_packed)
        .execute(&mut **tx)
        .await?;
    }
    Ok(id)
}

impl Database {
    pub async fn max_packet_bytes(&self) -> Result<u64, sqlx::Error> {
        sqlx::query_scalar("SELECT @@max_allowed_packet").fetch_one(&self.pool).await
    }

    pub async fn insert_request_log(&self, record: Record) -> Result<i64, sqlx::Error> {
        let mut ids = self.insert_request_logs(&[record]).await?;
        Ok(ids.remove(0))
    }

    pub async fn insert_request_logs(&self, records: &[Record]) -> Result<Vec<i64>, sqlx::Error> {
        let max_packet = self.max_packet_bytes().await? as usize;
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(records.len());
        for r in records {
            ids.push(insert_one(&mut tx, r, max_packet).await?);
        }
        tx.commit().await?;
        Ok(ids)
    }

    async fn delete_ids(&self, table: &str, column: &str, ids: Vec<i64>) -> Result<u64, sqlx::Error> {
        let mut removed = 0;
        for batch in ids.chunks(DELETE_BATCH) {
            let mut query = QueryBuilder::new(format!("DELETE FROM {table} WHERE {column} IN ("));
            let mut separated = query.separated(", ");
            for id in batch {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
            removed += query.build().execute(&self.pool).await?.rows_affected();
        }
        Ok(removed)
    }

    pub async fn prune_request_logs(&self, now_ms: i64, retention: &Retention) -> Result<u64, sqlx::Error> {
        let mut removed = 0;
        if retention.body_days > 0 {
            let ids: Vec<i64> = sqlx::query_scalar(
                "SELECT b.log_id FROM request_log_body b JOIN request_log l ON l.id = b.log_id \
                 WHERE l.started_at_ms < ?",
            )
            .bind(now_ms - i64::from(retention.body_days) * DAY_MS)
            .fetch_all(&self.pool)
            .await?;
            removed += self.delete_ids("request_log_body", "log_id", ids).await?;
        }
        if retention.max_body_bytes > 0 {
            let ids: Vec<i64> = sqlx::query_scalar(
                "SELECT log_id FROM (
                    SELECT log_id, SUM(size) OVER (ORDER BY log_id DESC) AS newest_first_total
                    FROM (
                        SELECT log_id,
                               COALESCE(LENGTH(client_request_packed), 0)
                             + COALESCE(LENGTH(upstream_request_packed), 0)
                             + COALESCE(LENGTH(response_packed), 0) AS size
                        FROM request_log_body
                    ) sizes
                ) totals WHERE newest_first_total > ?",
            )
            .bind(i64::try_from(retention.max_body_bytes).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?;
            removed += self.delete_ids("request_log_body", "log_id", ids).await?;
        }
        if retention.metadata_days > 0 {
            let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM request_log WHERE started_at_ms < ?")
                .bind(now_ms - i64::from(retention.metadata_days) * DAY_MS)
                .fetch_all(&self.pool)
                .await?;
            removed += self.delete_ids("request_log", "id", ids).await?;
        }
        Ok(removed)
    }

    pub async fn request_log_body_bytes(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT CAST(COALESCE(SUM(COALESCE(LENGTH(client_request_packed), 0)
                                    + COALESCE(LENGTH(upstream_request_packed), 0)
                                    + COALESCE(LENGTH(response_packed), 0)), 0) AS SIGNED)
             FROM request_log_body",
        )
        .fetch_one(&self.pool)
        .await
    }

    pub async fn list_request_logs(&self, limit: i64, filter: Filter<'_>) -> Result<Vec<Summary>, sqlx::Error> {
        let mut query = QueryBuilder::new(format!("SELECT {SUMMARY_COLUMNS} FROM request_log WHERE 1 = 1"));
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
        query.build_query_as::<Summary>().fetch_all(&self.pool).await
    }

    pub async fn request_log_detail(&self, id: i64) -> Result<Option<Detail>, sqlx::Error> {
        sqlx::query_as::<_, Detail>("SELECT * FROM request_log WHERE id = ?").bind(id).fetch_optional(&self.pool).await
    }

    pub async fn request_log_id(&self, request_id: &str) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM request_log WHERE request_id = ? ORDER BY id DESC LIMIT 1")
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn request_log_has_bodies(&self, id: i64) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM request_log_body WHERE log_id = ?)")
            .bind(id)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn request_log_bodies(&self, id: i64) -> Result<Option<Bodies>, sqlx::Error> {
        sqlx::query_as::<_, Bodies>(
            "SELECT client_request_packed, upstream_request_packed, response_packed \
             FROM request_log_body WHERE log_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }
}

#[cfg(test)]
mod tests;
