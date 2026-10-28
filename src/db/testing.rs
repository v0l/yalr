use super::Database;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const DEFAULT_SERVER: &str = "mysql://root:root@127.0.0.1:3307";
const PREFIX: &str = "yalr_test_";

pub struct DropOnLastUse {
    server: String,
    name: String,
}

impl Drop for DropOnLastUse {
    fn drop(&mut self) {
        let server = self.server.clone();
        let name = self.name.clone();
        let dropper = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            if let Ok(runtime) = runtime {
                runtime.block_on(drop_database(&server, &name));
            }
        });
        let _ = dropper.join();
    }
}

fn server() -> String {
    std::env::var("YALR_TEST_DATABASE_URL").unwrap_or_else(|_| DEFAULT_SERVER.to_string())
}

async fn admin(server: &str) -> Result<sqlx::MySqlConnection, sqlx::Error> {
    use sqlx::Connection;
    sqlx::MySqlConnection::connect(server).await
}

async fn drop_database(server: &str, name: &str) {
    if let Ok(mut conn) = admin(server).await {
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS `{name}`")).execute(&mut conn).await;
    }
}

fn pid_of(name: &str) -> Option<u32> {
    name.strip_prefix(PREFIX)?.split('_').next()?.parse().ok()
}

fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

async fn sweep_orphans(server: &str) {
    let Ok(mut conn) = admin(server).await else { return };
    let names: Vec<String> = sqlx::query_scalar(&format!("SHOW DATABASES LIKE '{PREFIX}%'"))
        .fetch_all(&mut conn)
        .await
        .unwrap_or_default();
    for name in names.into_iter().filter(|n| pid_of(n).is_some_and(|pid| !is_alive(pid))) {
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS `{name}`")).execute(&mut conn).await;
    }
}

fn next_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("{PREFIX}{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

impl Database {
    pub async fn test() -> Database {
        static SWEPT: OnceLock<()> = OnceLock::new();
        let server = server();
        if SWEPT.set(()).is_ok() {
            sweep_orphans(&server).await;
        }
        let name = next_name();
        let mut conn = admin(&server)
            .await
            .unwrap_or_else(|e| panic!("test MariaDB at {server} is unreachable ({e}); set YALR_TEST_DATABASE_URL"));
        sqlx::query(&format!("CREATE DATABASE `{name}`")).execute(&mut conn).await.unwrap();
        let guard = Arc::new(DropOnLastUse { server: server.clone(), name: name.clone() });
        let mut db = Database::new(&format!("{}/{name}", server.trim_end_matches('/'))).await.unwrap();
        db._test_guard = Some(guard);
        db
    }

    pub fn test_url(&self) -> Option<String> {
        self._test_guard.as_ref().map(|g| format!("{}/{}", g.server.trim_end_matches('/'), g.name))
    }
}

impl Database {
    pub async fn price_model(&self, model_name: &str, is_advertised: bool) -> super::ModelPricingRow {
        self.create_model_pricing(super::NewModelPricing {
            model_name,
            is_advertised,
            is_free: false,
            price_per_1m_input_sats: None,
            price_per_1m_output_sats: None,
            price_per_1m_cache_read_sats: None,
            price_per_1m_cache_write_sats: None,
            price_per_request_sats: None,
            context_window: None,
            max_output_tokens: None,
        })
        .await
        .unwrap()
    }

    pub async fn add_test_user(&self, username: &str, is_admin: bool) -> super::User {
        self.create_user(super::NewUser {
            username: Some(username),
            password_hash: None,
            external_id: None,
            user_type: super::UserType::Internal,
            is_admin,
        })
        .await
        .unwrap()
    }

    pub async fn execute_raw(&self, sql: &str) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(sql).execute(&self.pool).await?.rows_affected())
    }
}
