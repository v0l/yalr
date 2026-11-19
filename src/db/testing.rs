use super::Database;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

const DEFAULT_SERVER: &str = "mysql://root:root@127.0.0.1:3307";
const TEMPLATE_PREFIX: &str = "yalr_tpl_";
const TEMPLATE_LOCK: &str = "yalr_tpl_build";
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
        let ids: Vec<i64> = sqlx::query_scalar("SELECT ID FROM information_schema.PROCESSLIST WHERE DB = ?")
            .bind(name)
            .fetch_all(&mut conn)
            .await
            .unwrap_or_default();
        for id in ids {
            let _ = sqlx::query(&format!("KILL {id}")).execute(&mut conn).await;
        }
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

fn template_name() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for migration in sqlx::migrate!("./migrations").iter() {
        migration.version.hash(&mut hasher);
        migration.checksum.hash(&mut hasher);
    }
    format!("{TEMPLATE_PREFIX}{:016x}", hasher.finish())
}

async fn build_template(server: &str, name: &str) -> Vec<String> {
    let mut conn = admin(server).await.unwrap();
    let _: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK(?, 600)").bind(TEMPLATE_LOCK).fetch_one(&mut conn).await.unwrap();
    let exists: Vec<String> = sqlx::query_scalar(&format!("SHOW DATABASES LIKE '{name}'")).fetch_all(&mut conn).await.unwrap();
    if exists.is_empty() {
        sqlx::query(&format!("CREATE DATABASE `{name}`")).execute(&mut conn).await.unwrap();
        let built = Database::new(&format!("{}/{name}", server.trim_end_matches('/'))).await;
        if let Err(e) = built {
            drop_database(server, name).await;
            panic!("migrating the template database failed: {e}");
        }
    }
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
    )
    .bind(name)
    .fetch_all(&mut conn)
    .await
    .unwrap();
    let mut ddl = Vec::new();
    for table in tables {
        let row: (String, String) = sqlx::query_as(&format!("SHOW CREATE TABLE `{name}`.`{table}`")).fetch_one(&mut conn).await.unwrap();
        ddl.push(row.1);
    }
    let _ = sqlx::query("SELECT RELEASE_LOCK(?)").bind(TEMPLATE_LOCK).execute(&mut conn).await;
    ddl
}

async fn template_ddl(server: &str) -> &'static (String, Vec<String>) {
    static TEMPLATE: OnceLock<(String, Vec<String>)> = OnceLock::new();
    if let Some(t) = TEMPLATE.get() {
        return t;
    }
    let name = template_name();
    let ddl = build_template(server, &name).await;
    TEMPLATE.get_or_init(|| (name, ddl))
}

async fn clone_template(conn: &mut sqlx::MySqlConnection, server: &str, name: &str) {
    let (template, ddl) = template_ddl(server).await;
    sqlx::query(&format!("USE `{name}`")).execute(&mut *conn).await.unwrap();
    sqlx::query("SET foreign_key_checks = 0").execute(&mut *conn).await.unwrap();
    for statement in ddl {
        sqlx::query(statement).execute(&mut *conn).await.unwrap();
    }
    sqlx::query(&format!("INSERT INTO _sqlx_migrations SELECT * FROM `{template}`._sqlx_migrations"))
        .execute(&mut *conn)
        .await
        .unwrap();
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
        clone_template(&mut conn, &server, &name).await;
        drop(conn);
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
            owner_user_id: None,
            provider_id: None,
            currency: "sats",
            fiat: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;
    use std::time::Duration;

    fn target(db: &Database) -> (String, String) {
        let guard = db._test_guard.as_ref().unwrap();
        (guard.server.clone(), guard.name.clone())
    }

    async fn database_exists(server: &str, name: &str) -> bool {
        let mut conn = admin(server).await.unwrap();
        let found: Vec<String> = sqlx::query_scalar(&format!("SHOW DATABASES LIKE '{name}'")).fetch_all(&mut conn).await.unwrap();
        !found.is_empty()
    }

    async fn schema_of(server: &str, name: &str) -> (Vec<(String, String, String, String, String, String)>, Vec<(String, String, i64, String, String)>) {
        let mut conn = admin(server).await.unwrap();
        let columns = sqlx::query_as(
            "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, CAST(COALESCE(COLUMN_DEFAULT, 'NULL') AS CHAR), EXTRA \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION",
        )
        .bind(name)
        .fetch_all(&mut conn)
        .await
        .unwrap();
        let indexes = sqlx::query_as(
            "SELECT TABLE_NAME, INDEX_NAME, CAST(SEQ_IN_INDEX AS SIGNED), COLUMN_NAME, CAST(NON_UNIQUE AS CHAR) \
             FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX",
        )
        .bind(name)
        .fetch_all(&mut conn)
        .await
        .unwrap();
        (columns, indexes)
    }

    async fn foreign_keys_of(server: &str, name: &str) -> Vec<(String, String, String, String, String, String)> {
        let mut conn = admin(server).await.unwrap();
        sqlx::query_as(
            "SELECT c.TABLE_NAME, c.CONSTRAINT_NAME, k.COLUMN_NAME, c.REFERENCED_TABLE_NAME, c.DELETE_RULE, c.UPDATE_RULE \
             FROM information_schema.REFERENTIAL_CONSTRAINTS c \
             JOIN information_schema.KEY_COLUMN_USAGE k ON k.CONSTRAINT_SCHEMA = c.CONSTRAINT_SCHEMA AND k.CONSTRAINT_NAME = c.CONSTRAINT_NAME AND k.TABLE_NAME = c.TABLE_NAME \
             WHERE c.CONSTRAINT_SCHEMA = ? ORDER BY 1, 2, 3",
        )
        .bind(name)
        .fetch_all(&mut conn)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn cloned_database_schema_equals_a_freshly_migrated_one() {
        let cloned = Database::test().await;
        let (server, cloned_name) = target(&cloned);
        let fresh_name = format!("{PREFIX}{}_fresh", std::process::id());
        let mut conn = admin(&server).await.unwrap();
        sqlx::query(&format!("CREATE DATABASE `{fresh_name}`")).execute(&mut conn).await.unwrap();
        let fresh = Database::new(&format!("{}/{fresh_name}", server.trim_end_matches('/'))).await.unwrap();

        let expected = schema_of(&server, &fresh_name).await;
        let actual = schema_of(&server, &cloned_name).await;
        assert!(!expected.0.is_empty());
        assert_eq!(expected, actual);
        assert_eq!(foreign_keys_of(&server, &fresh_name).await, foreign_keys_of(&server, &cloned_name).await);

        let applied = |db: &Database| {
            let pool = db.pool.clone();
            async move {
                sqlx::query_as::<_, (i64, String)>("SELECT version, HEX(checksum) FROM _sqlx_migrations ORDER BY version")
                    .fetch_all(&pool)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(applied(&fresh).await, applied(&cloned).await);
        drop(fresh);
        drop_database(&server, &fresh_name).await;
    }

    #[tokio::test]
    async fn drop_database_succeeds_while_a_connection_holds_an_open_transaction() {
        let db = Database::test().await;
        let bystander = Database::test().await;
        let (server, name) = target(&db);
        let (_, bystander_name) = target(&bystander);

        let mut holder = sqlx::MySqlConnection::connect(&db.test_url().unwrap()).await.unwrap();
        sqlx::query("BEGIN").execute(&mut holder).await.unwrap();
        sqlx::query("INSERT INTO model_pricing (model_name) VALUES ('held')").execute(&mut holder).await.unwrap();

        tokio::time::timeout(Duration::from_secs(120), drop_database(&server, &name))
            .await
            .expect("drop_database blocked on a lingering connection");

        assert!(!database_exists(&server, &name).await);
        assert!(database_exists(&server, &bystander_name).await);
        assert!(bystander.execute_raw("SELECT 1").await.is_ok());
    }

    #[tokio::test]
    async fn drop_database_ignores_connections_that_vanish_and_missing_databases() {
        let db = Database::test().await;
        let (server, name) = target(&db);
        let conn = sqlx::MySqlConnection::connect(&db.test_url().unwrap()).await.unwrap();
        drop(conn);
        tokio::time::timeout(Duration::from_secs(120), async {
            drop_database(&server, &name).await;
            drop_database(&server, &name).await;
        })
        .await
        .unwrap();
        assert!(!database_exists(&server, &name).await);
    }
}
