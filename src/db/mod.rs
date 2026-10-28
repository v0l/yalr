use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};
use std::str::FromStr;
use std::time::Duration;

mod anonymous;
mod auth_cache;
mod billing_exempt;
mod import;
mod metrics_history;
mod modalities;
mod nostr_identity;
mod payments;
mod pricing;
mod providers;
mod request_log;
mod routing;
pub mod testing;
mod types;
mod usage;
mod users;

pub use anonymous::{random_hex, AccountInvoice, PublicInvoice};
pub use import::ImportReport;
pub use modalities::DeclaredModalities;
pub use request_log::{Bodies, Detail, Filter, Record, Retention, Summary};
pub use payments::{Debit, LedgerEntry};
pub use types::*;
pub use users::UserChanges;
pub use usage::{ApiKeyUsage, HourUsage, ModelUsage, UsageReport, UsageScope};

const DB_MAX_CONNECTIONS: u32 = 32;

pub type Timestamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone)]
pub struct Database {
    pool: MySqlPool,
    pub log_writer: crate::request_log::recorder::Writer,
    auth_cache: auth_cache::AuthCache,
    _test_guard: Option<std::sync::Arc<testing::DropOnLastUse>>,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = connect(database_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self::from_pool(pool))
    }

    fn from_pool(pool: MySqlPool) -> Self {
        let mut db = Self {
            pool,
            log_writer: crate::request_log::recorder::Writer::disconnected(),
            auth_cache: Default::default(),
            _test_guard: None,
        };
        db.log_writer = crate::request_log::recorder::Writer::spawn(db.clone());
        db
    }

    pub async fn close(&self) -> Result<(), sqlx::Error> {
        self.pool.close().await;
        Ok(())
    }
}

async fn connect(database_url: &str) -> Result<MySqlPool, sqlx::Error> {
    let options = MySqlConnectOptions::from_str(database_url)?
        .charset("utf8mb4")
        .timezone(Some("+00:00".to_string()));
    MySqlPoolOptions::new()
        .max_connections(DB_MAX_CONNECTIONS)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options)
        .await
}

#[cfg(test)]
mod tests;
