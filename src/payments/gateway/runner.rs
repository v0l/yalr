use super::GatewayService;
use std::sync::Arc;
use std::time::Duration;

const TICK: Duration = Duration::from_secs(30);
const PENDING_BATCH: i64 = 50;
const UNSENT_AFTER_SECS: i64 = 3600;

impl GatewayService {
    pub async fn reconcile_stale(&self) -> usize {
        let stale = match self.db.stale_pending_gateway_payments(self.settings.reconcile_after_secs, PENDING_BATCH).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "Could not list pending gateway payments");
                return 0;
            }
        };
        let mut settled = 0;
        for payment in stale {
            let Some(remote) = payment.gateway_payment_id.as_deref() else { continue };
            match self.reconcile(remote).await {
                Ok(Some(_)) => settled += 1,
                Ok(None) => {}
                Err(e) => tracing::warn!(payment = %payment.public_id, error = %e, "Gateway payment reconciliation failed"),
            }
        }
        settled
    }

    pub async fn run_auto_reloads(&self) -> usize {
        let due = match self.db.auto_reloads_below_threshold().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "Could not list auto-reload candidates");
                return 0;
            }
        };
        let mut charged = 0;
        for candidate in due {
            let claim = self.db.claim_auto_reload(candidate.user_id, &candidate.currency, self.settings.auto_reload_cooldown_secs).await;
            let (order, settings) = match claim {
                Ok(Some(claimed)) => claimed,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = %e, "Auto-reload claim failed");
                    continue;
                }
            };
            charged += 1;
            if let Err(e) = self.charge_auto_reload(&order, &settings).await {
                tracing::warn!(payment = %order.public_id, error = %e, "Auto-reload charge failed");
            }
        }
        charged
    }
}

pub fn spawn_runner(service: Arc<GatewayService>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            service.reconcile_stale().await;
            let _ = service.db.expire_unsent_gateway_payments(UNSENT_AFTER_SECS).await;
            service.run_auto_reloads().await;
        }
    })
}
