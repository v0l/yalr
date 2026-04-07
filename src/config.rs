use crate::db::Database;
use crate::metrics::MetricsStore;
use crate::providers::openai::OpenAiProvider;
use crate::router::engine::Router;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DatabaseConfig {
    pub url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
            },
            database: DatabaseConfig {
                url: "sqlite:llm_router.db?mode=rwc".to_string(),
            },
        }
    }
}

#[derive(Clone)]
pub struct AppConfig {
    pub db: Arc<Database>,
    pub router: Arc<Router>,
}

impl AppConfig {
    pub async fn load(metrics_store: MetricsStore) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config: Config = config::Config::builder()
            .add_source(config::File::with_name("config").required(false).format(config::FileFormat::Yaml))
            .build()?
            .try_deserialize()?;

        let db = Arc::new(Database::new(&config.database.url).await?);
        db.run_migrations().await?;

        let router = Arc::new(Router::new(
            Box::new(crate::router::strategies::round_robin::RoundRobinStrategy::new()),
            metrics_store,
        ));

        Ok(Self { db, router })
    }

    pub async fn load_providers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let providers = sqlx::query_as::<_, ProviderRecord>(
            "SELECT id, name, slug, base_url, api_key FROM providers",
        )
        .fetch_all(&self.db.pool)
        .await?;

        for provider_record in providers {
            let provider = Arc::new(OpenAiProvider::new(
                &provider_record.name,
                Some(&provider_record.slug),
                &provider_record.base_url,
                provider_record.api_key.as_deref(),
            ));
            self.router.add_provider(provider).await;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ProviderRecord {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub base_url: String,
    pub api_key: Option<String>,
}
