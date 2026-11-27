pub mod balance;
pub mod account;
pub mod anonymous;
pub mod biller;
pub mod gateway;
pub mod guard;
pub mod instructions;
pub mod pricing;
pub mod provider;
pub mod routstr;
#[cfg(test)]
mod failover_billing_tests;
#[cfg(test)]
mod settlement_tests;

use std::sync::Arc;
use crate::db::Database;
use crate::config::PaymentsConfig;
use provider::PaymentProvider;

#[derive(Clone)]
pub struct PaymentsState {
    pub config: PaymentsConfig,
    pub balance_service: Arc<balance::BalanceService>,
    pub pricing_resolver: Arc<pricing::PricingResolver>,
    pub gateway: Option<Arc<gateway::GatewayService>>,
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

        let gateway = match payments_config.gateway.clone() {
            None => None,
            Some(settings) => match gateway::GatewaySecrets::from_env() {
                Some(secrets) => {
                    let service = Arc::new(gateway::GatewayService::new(settings, secrets, db.clone())?);
                    gateway::spawn_runner(service.clone());
                    tracing::info!(mode = service.mode(), "Payment gateway enabled");
                    Some(service)
                }
                None => {
                    tracing::warn!("payments.gateway is configured but GATEWAY_API_KEY or GATEWAY_WEBHOOK_SECRET is missing, top-ups disabled");
                    None
                }
            },
        };

        Ok(Self { config: payments_config, balance_service, pricing_resolver, gateway })
    }

    pub fn provider(&self) -> Option<Arc<dyn PaymentProvider>> {
        self.gateway.clone().map(|g| g as Arc<dyn PaymentProvider>)
    }
}
