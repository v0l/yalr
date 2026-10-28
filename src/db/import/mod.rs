use super::Database;
use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, MySql, MySqlConnection, QueryBuilder, Row, TypeInfo, ValueRef};
use std::str::FromStr;

const BATCH: usize = 500;
const BATCH_BYTES: usize = 4 * 1024 * 1024;
const PACKET_HEADROOM: usize = 64 * 1024;

const TABLES: &[&str] = &[
    "providers",
    "models",
    "model_providers",
    "routing_config",
    "routing_config_providers",
    "users",
    "api_keys",
    "user_model_permissions",
    "billing_exempt_users",
    "user_balances",
    "balance_transactions",
    "lightning_invoices",
    "model_pricing",
    "api_key_usage",
    "metrics_history",
    "nostr_identity",
    "server_secret",
    "request_log",
    "request_log_body",
];

const TIMESTAMP_COLUMNS: &[&str] = &["created_at", "updated_at", "expires_at", "paid_at", "hour"];

#[derive(Debug, Default)]
pub struct ImportReport {
    pub tables: Vec<(String, u64)>,
    pub skipped: Vec<(String, u64)>,
}

impl Value {
    fn size(&self) -> usize {
        match self {
            Value::Text(v) => v.len(),
            Value::Blob(v) => v.len(),
            _ => 8,
        }
    }
}

enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Time(DateTime<Utc>),
}

fn parse_timestamp(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(text) {
        return Some(t.with_timezone(&Utc));
    }
    ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(text, f).ok())
        .map(|t| t.and_utc())
}

fn read(row: &SqliteRow, index: usize, name: &str) -> Result<Value, sqlx::Error> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok(Value::Null);
    }
    let value = match raw.type_info().name() {
        "INTEGER" | "BOOLEAN" => Value::Int(row.try_get(index)?),
        "REAL" => Value::Real(row.try_get(index)?),
        "BLOB" => Value::Blob(row.try_get(index)?),
        _ => {
            let text: String = row.try_get(index)?;
            match TIMESTAMP_COLUMNS.contains(&name).then(|| parse_timestamp(&text)) {
                Some(Some(time)) => Value::Time(time),
                Some(None) => {
                    return Err(sqlx::Error::Decode(format!("unparsable timestamp {name} = {text:?}").into()))
                }
                None => Value::Text(text),
            }
        }
    };
    Ok(value)
}

fn push(query: &mut sqlx::query_builder::Separated<'_, '_, MySql, &str>, value: Value) {
    match value {
        Value::Null => query.push_bind(None::<i64>),
        Value::Int(v) => query.push_bind(v),
        Value::Real(v) => query.push_bind(v),
        Value::Text(v) => query.push_bind(v),
        Value::Blob(v) => query.push_bind(v),
        Value::Time(v) => query.push_bind(v),
    };
}

async fn open_sqlite(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(path)?.read_only(false).create_if_missing(false);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
    sqlx::migrate!("./migrations-sqlite").run(&pool).await?;
    Ok(pool)
}

async fn columns(source: &SqlitePool, table: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})")).fetch_all(source).await?;
    rows.iter().map(|r| r.try_get::<String, _>("name")).collect()
}

async fn insert_rows(
    target: &mut MySqlConnection,
    table: &str,
    list: &str,
    rows: Vec<Vec<Value>>,
) -> Result<u64, sqlx::Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut insert = QueryBuilder::<MySql>::new(format!("INSERT INTO {table} ({list}) "));
    insert.push_values(rows, |mut b, row| {
        for value in row {
            push(&mut b, value);
        }
    });
    Ok(insert.build().execute(&mut *target).await?.rows_affected())
}

impl Database {
    async fn is_empty(&self) -> Result<bool, sqlx::Error> {
        for table in TABLES {
            let any: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM {table})"))
                .fetch_one(&self.pool)
                .await?;
            if any {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn copy_table(
        &self,
        target: &mut MySqlConnection,
        source: &SqlitePool,
        table: &str,
        max_packet: usize,
    ) -> Result<(u64, u64), sqlx::Error> {
        let columns = columns(source, table).await?;
        let list = columns.join(", ");
        let mut copied = 0;
        let mut skipped = 0;
        let mut after = i64::MIN;
        loop {
            let rows = sqlx::query(&format!(
                "SELECT rowid AS _import_rowid, {list} FROM {table} WHERE rowid > ? ORDER BY rowid LIMIT ?"
            ))
            .bind(after)
            .bind(BATCH as i64)
            .fetch_all(source)
            .await?;
            let Some(last) = rows.last() else { break };
            after = last.try_get("_import_rowid")?;
            let mut batch = Vec::with_capacity(rows.len());
            let mut batch_bytes = 0;
            for row in &rows {
                let mut out = Vec::with_capacity(columns.len());
                for (i, column) in row.columns().iter().enumerate().skip(1) {
                    out.push(read(row, i, column.name())?);
                }
                let size: usize = out.iter().map(Value::size).sum();
                if size + PACKET_HEADROOM > max_packet {
                    let rowid: i64 = row.try_get("_import_rowid")?;
                    tracing::warn!(table, rowid, bytes = size, "Row exceeds max_allowed_packet, skipped");
                    skipped += 1;
                    continue;
                }
                if !batch.is_empty() && batch_bytes + size > BATCH_BYTES {
                    copied += insert_rows(target, table, &list, std::mem::take(&mut batch)).await?;
                    batch_bytes = 0;
                }
                batch_bytes += size;
                batch.push(out);
            }
            copied += insert_rows(target, table, &list, batch).await?;
        }
        Ok((copied, skipped))
    }

    pub async fn import_sqlite(&self, sqlite_url: &str) -> Result<ImportReport, sqlx::Error> {
        if !self.is_empty().await? {
            return Err(sqlx::Error::Protocol("target database already has data; import needs an empty one".into()));
        }
        let source = open_sqlite(sqlite_url).await?;
        let mut report = ImportReport::default();
        let max_packet = self.max_packet_bytes().await? as usize;
        let mut target = self.pool.acquire().await?;
        sqlx::query("SET SESSION foreign_key_checks = 0").execute(&mut *target).await?;
        for table in TABLES {
            let (copied, skipped) = self.copy_table(&mut target, &source, table, max_packet).await?;
            tracing::info!(table, rows = copied, skipped, "Imported table");
            report.tables.push((table.to_string(), copied));
            if skipped > 0 {
                report.skipped.push((table.to_string(), skipped));
            }
        }
        sqlx::query("SET SESSION foreign_key_checks = 1").execute(&mut *target).await?;
        source.close().await;
        Ok(report)
    }
}

#[cfg(test)]
mod tests;
