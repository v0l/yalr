use sqlx::SqlitePool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(database_url).await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }
}

pub type DbPool = Arc<SqlitePool>;
