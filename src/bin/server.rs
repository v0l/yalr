use yalr::{api, config, metrics};
use std::env;
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

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("{}:{}", host, port);

    tracing::info!(addr = %addr, "Starting YALR");

    api::server::run(config, &addr, emitter, metrics_store).await?;

    Ok(())
}
