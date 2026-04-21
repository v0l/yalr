use yalr::{api, config, metrics};
use std::env;
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let (emitter, _receiver) = metrics::MetricsEmitter::new(10000);
    let metrics_store = metrics::MetricsStore::new(emitter.clone(), 10000);
    
    let config = config::AppConfig::load(metrics_store.clone()).await.expect("Failed to load config");
    config.load_providers().await.expect("Failed to load providers");

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("{}:{}", host, port);

    tracing::info!(addr = %addr, "Starting YALR");

    api::server::run(config, &addr, emitter, metrics_store).await?;

    Ok(())
}
