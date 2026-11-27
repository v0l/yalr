use super::{InvoiceState, LndService, SETTLE_CURSOR};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

const REPAIR_BATCH: i64 = 200;
const RESUBSCRIBE_AFTER: Duration = Duration::from_secs(5);

impl LndService {
    pub async fn repair(&self) -> usize {
        let pending = match self.db.pending_lnd_payments(REPAIR_BATCH).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "Could not list pending LND invoices");
                return 0;
            }
        };
        let mut closed = 0;
        for payment in pending {
            match self.refresh_one(&payment).await {
                Ok(Some(_)) => closed += 1,
                Ok(None) => {}
                Err(e) => tracing::warn!(payment = %payment.public_id, error = %e, "LND invoice lookup failed"),
            }
        }
        closed
    }

    pub async fn follow(&self) -> Result<(), String> {
        let cursor = self.db.payment_cursor(SETTLE_CURSOR).await.map_err(|e| e.to_string())?;
        let mut stream = self.node.subscribe(cursor).await?;
        tracing::info!(settle_index = cursor, "Following LND invoices");
        while let Some(item) = stream.next().await {
            let event = item?;
            if !matches!(event.state, InvoiceState::Settled | InvoiceState::Canceled) {
                continue;
            }
            self.apply(&event).await.map_err(|e| e.to_string())?;
            if event.state == InvoiceState::Settled && event.settle_index > 0 {
                self.db.advance_payment_cursor(SETTLE_CURSOR, event.settle_index).await.map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

pub fn spawn_watcher(service: Arc<LndService>) -> tokio::task::JoinHandle<()> {
    let repairer = service.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(repairer.settings.repair_interval_secs.max(1)));
        loop {
            tick.tick().await;
            let closed = repairer.repair().await;
            if closed > 0 {
                tracing::info!(closed, "Repaired LND invoices from lookups");
            }
        }
    });
    spawn_follower(service, RESUBSCRIBE_AFTER)
}

pub fn spawn_follower(service: Arc<LndService>, pause: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match service.follow().await {
                Ok(()) => tracing::warn!("LND invoice stream ended, resubscribing"),
                Err(e) => tracing::warn!(error = %e, "LND invoice stream failed, resubscribing"),
            }
            tokio::time::sleep(pause).await;
            service.repair().await;
        }
    })
}
