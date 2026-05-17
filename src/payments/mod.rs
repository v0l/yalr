pub mod balance;
pub mod biller;
pub mod guard;
pub mod lightning;
pub mod pricing;
pub mod routstr;

use std::sync::Arc;
use crate::db::Database;
use crate::config::PaymentsConfig;

/// Holds all initialized payments sub-services.
/// Created at startup when payments.enabled is true.
#[derive(Clone)]
pub struct PaymentsState {
    pub config: PaymentsConfig,
    pub balance_service: Arc<balance::BalanceService>,
    pub lightning_service: Option<Arc<lightning::LightningService>>,
    pub pricing_resolver: Arc<pricing::PricingResolver>,
    /// Held to keep the background settlement listener alive.
    #[allow(dead_code)]
    pub settlement_listener: Option<Arc<tokio::task::JoinHandle<()>>>,
}

impl PaymentsState {
    pub async fn new(
        payments_config: PaymentsConfig,
        db: Arc<Database>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let balance_service = Arc::new(balance::BalanceService::new(db.clone()));
        let pricing_resolver = Arc::new(pricing::PricingResolver::new(
            db.clone(),
            payments_config.default_pricing.clone(),
        ));

        let mut settlement_listener = None;

        let lightning_service = if let Some(ref lnd_config) = payments_config.lnd {
            payments_rs::lightning::setup_crypto_provider();
            let lnd = payments_rs::lightning::LndNode::new(
                &lnd_config.url,
                std::path::Path::new(&lnd_config.tls_cert_path),
                std::path::Path::new(&lnd_config.macaroon_path),
            ).await?;

            let ls = Arc::new(lightning::LightningService::new(
                lnd,
                db.clone(),
            ));

            // Start background settlement listener
            let handle = ls.start_settlement_listener(balance_service.clone());
            settlement_listener = Some(Arc::new(handle));

            Some(ls)
        } else {
            None
        };

        Ok(Self {
            config: payments_config,
            balance_service,
            lightning_service,
            pricing_resolver,
            settlement_listener,
        })
    }
}
