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

    let metrics_store = std::sync::Arc::new(metrics::MetricsStore::with_health_config(
        10000,
        Some(metrics::HealthConfig {
            thresholds: config::Config::load_file().router.health,
            ..Default::default()
        }),
    ));
    let emitter = metrics_store.emitter().clone();
    
    let config = config::AppConfig::load(metrics_store.clone()).await.expect("Failed to load config");
    config.load_providers().await.expect("Failed to load providers");

    let background_db = config.db.clone();
    let background_metrics = metrics_store.clone();
    tokio::spawn(async move {
        background_metrics.set_db(background_db.clone());
        background_metrics.load_history_from_db().await;
        background_metrics.start_history_snapshots(300);

        let log_settings = yalr::request_log::settings::get();
        if log_settings.enabled {
            let retention = yalr::db::Retention {
                body_days: log_settings.body_retention_days,
                metadata_days: log_settings.metadata_retention_days,
                max_body_bytes: log_settings.max_body_bytes,
            };
            yalr::request_log::recorder::prune_now(&background_db, &retention).await;
            yalr::request_log::recorder::spawn_pruner((*background_db).clone(), retention);
        }
    });

    if config.payments_config.as_ref().is_some_and(|p| p.enabled) {
        yalr::payments::anonymous::spawn_unpaid_pruner(config.db.clone());
    }

    yalr::discovery::profile::spawn_refresher(
        config.db.clone(),
        yalr::discovery::relays(config.discovery_config.as_ref()),
    );

    yalr::verify::spawn_scheduler(config.db.clone(), config.router.clone());

    let prune_db = config.db.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let cutoff = chrono::Utc::now().timestamp_millis() - 8 * 86_400_000;
            let _ = prune_db.prune_health_checks(cutoff).await;
        }
    });

    let addr = format!("{}:{}", config.host, config.port);

    tracing::info!(addr = %addr, "Starting YALR");

    api::server::run(config, &addr, emitter, metrics_store).await?;

    Ok(())
}
