use sqlx::SqlitePool;
use std::sync::Arc;

pub mod schema;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), sqlx::Error> {
        sqlx::query(schema::CREATE_PROVIDERS_TABLE)
            .execute(&self.pool)
            .await?;
        sqlx::query(schema::CREATE_MODELS_TABLE)
            .execute(&self.pool)
            .await?;
        sqlx::query(schema::CREATE_MODEL_PROVIDERS_TABLE)
            .execute(&self.pool)
            .await?;
        sqlx::query(schema::CREATE_ROUTING_CONFIG_TABLE)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

pub type DbPool = Arc<SqlitePool>;
