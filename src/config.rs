use crate::db::Database;
use crate::metrics::MetricsStore;
use crate::router::engine::Router;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: Option<AuthConfig>,
    pub payments: Option<PaymentsConfig>,
    #[serde(default)]
    pub anthropic: AnthropicConfig,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AnthropicConfig {
    /// Inject Anthropic prompt-cache breakpoints (system/tools + the growing
    /// conversation prefix). On by default; set to `false` for purely one-shot
    /// traffic where the cache-write premium wouldn't pay off.
    #[serde(default = "default_true")]
    pub prompt_cache: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self { prompt_cache: true }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_admin_ui_path")]
    pub admin_ui_path: String,
}

fn default_admin_ui_path() -> String {
    "/app/admin/dist".to_string()
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub allowed_pubkeys: Option<Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
                admin_ui_path: default_admin_ui_path(),
            },
            database: DatabaseConfig {
                url: "sqlite:llm_router.db?mode=rwc".to_string(),
            },
            auth: None,
            payments: None,
            anthropic: AnthropicConfig::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[derive(Default)]
pub struct PaymentsConfig {
    pub enabled: bool,
    #[serde(default)]
    pub default_pricing: DefaultPricingConfig,
    pub lnd: Option<LndConfig>,
}


#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct DefaultPricingConfig {
    #[serde(default)]
    pub price_per_1m_input_sats: i64,
    #[serde(default)]
    pub price_per_1m_output_sats: i64,
    #[serde(default)]
    pub price_per_request_sats: i64,
    #[serde(default = "DefaultPricingConfig::default_context_window")]
    pub context_window: i32,
    #[serde(default = "DefaultPricingConfig::default_max_output_tokens")]
    pub max_output_tokens: i32,
}

impl Default for DefaultPricingConfig {
    fn default() -> Self {
        Self {
            price_per_1m_input_sats: 0,
            price_per_1m_output_sats: 0,
            price_per_request_sats: 0,
            context_window: Self::default_context_window(),
            max_output_tokens: Self::default_max_output_tokens(),
        }
    }
}

impl DefaultPricingConfig {
    fn default_context_window() -> i32 { 8192 }
    fn default_max_output_tokens() -> i32 { 4096 }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct LndConfig {
    pub url: String,
    pub tls_cert_path: String,
    pub macaroon_path: String,
}

#[derive(Clone)]
pub struct AppConfig {
    pub db: Arc<Database>,
    pub router: Arc<Router>,
    pub auth_config: crate::auth::nip98::AuthConfig,
    pub payments_config: Option<PaymentsConfig>,
    pub admin_ui_path: String,
    pub host: String,
    pub port: u16,
}

impl AppConfig {
    pub async fn load(metrics_store: std::sync::Arc<MetricsStore>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let config: Config = config::Config::builder()
            .add_source(config::File::with_name("config").required(false).format(config::FileFormat::Yaml))
            .build()?
            .try_deserialize()?;

        let db = Arc::new(Database::new(&config.database.url).await?);

        let router = Arc::new(Router::new(
            (*metrics_store).clone(),
            db.clone(),
        ));

        if let Err(e) = router.reload_config().await {
            tracing::warn!(error = %e, "Failed to reload router config from DB, starting with empty config");
        }

        let auth_config = config.auth.map(|a| {
            crate::auth::nip98::AuthConfig {
                enabled: a.enabled,
                allowed_pubkeys: a.allowed_pubkeys
                    .map(|keys| keys.into_iter().collect())
                    .unwrap_or_default(),
            }
        }).unwrap_or_default();

        // Apply provider-level feature toggles sourced from the config file.
        crate::providers::anthropic::set_prompt_cache_enabled(config.anthropic.prompt_cache);

        let payments_config = config.payments;
        let admin_ui_path = config.server.admin_ui_path.clone();
        let host = config.server.host.clone();
        let port = config.server.port;

        Ok(Self { db, router, auth_config, payments_config, admin_ui_path, host, port })
    }

    pub async fn load_providers(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.router.reload_config().await
    }
}
