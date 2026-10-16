use yalr::{api, config, metrics};
use clap::Parser;

#[derive(Parser)]
#[command(name = "yalr-server")]
#[command(author = "YALR")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "YALR - LLM Router", long_about = None)]
struct Args {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _args = Args::parse();
    
    // Clap will automatically handle --version, --help, etc.
    
    tracing_subscriber::fmt::init();

    let metrics_store = std::sync::Arc::new(metrics::MetricsStore::new(10000));
    let emitter = metrics_store.emitter().clone();
    
    let config = config::AppConfig::load(metrics_store.clone()).await.expect("Failed to load config");
    config.load_providers().await.expect("Failed to load providers");

    // Wire DB pool into metrics store for history persistence
    metrics_store.set_db_pool(config.db.pool.clone());
    metrics_store.load_history_from_db().await;
    metrics_store.start_history_snapshots(300); // 5-minute intervals

    let log_settings = yalr::request_log::settings::get();
    if log_settings.enabled {
        yalr::request_log::recorder::spawn_pruner(
            config.db.pool.clone(),
            log_settings.body_retention_days,
            log_settings.metadata_retention_days,
        );
    }

    let addr = format!("{}:{}", config.host, config.port);

    tracing::info!(addr = %addr, "Starting YALR");

    api::server::run(config, &addr, emitter, metrics_store).await?;

    Ok(())
}
