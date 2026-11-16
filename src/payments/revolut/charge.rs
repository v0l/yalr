use super::{upstream, RevolutError, RevolutService, MILLI_PER_MINOR};
use crate::db::auto_reload::AutoReload;
use crate::db::fiat_orders::FiatOrder;
use payments_rs::fiat::{PayOrderRequest, RevolutCustomer, RevolutSavedPaymentMethodType, SavedPaymentMethodRef};

impl RevolutService {
    pub async fn charge_auto_reload(&self, order: &FiatOrder, settings: &AutoReload) -> Result<Option<FiatOrder>, RevolutError> {
        let Some(method) = self.db.payment_method(order.user_id, settings.payment_method_id).await? else {
            self.db.fail_unsent_fiat_order(order.id, "saved card no longer exists").await?;
            self.db.note_auto_reload_outcome(order.user_id, &order.currency, false).await?;
            return Ok(None);
        };
        let customer_id = self.secrets.decrypt(&method.external_customer_id).map_err(upstream)?;
        let minor = order.amount_milli / MILLI_PER_MINOR;
        let customer = RevolutCustomer { id: Some(customer_id), ..Default::default() };
        let created = match self.send_order(order, minor, Some(customer), false).await {
            Ok(created) => created,
            Err(e) => {
                self.db.fail_unsent_fiat_order(order.id, "processor rejected the order").await?;
                self.db.note_auto_reload_outcome(order.user_id, &order.currency, false).await?;
                return Err(e);
            }
        };
        let pay = PayOrderRequest {
            saved_payment_method: SavedPaymentMethodRef {
                kind: RevolutSavedPaymentMethodType::Card.as_str().to_string(),
                id: method.external_id,
                initiator: "merchant".to_string(),
            },
        };
        if let Err(e) = self.api.pay_order(&created.id, pay).await {
            tracing::warn!(order = %order.public_id, error = %e, "Off-session charge was refused");
            self.db.close_fiat_order(&created.id, crate::db::fiat_orders::FAILED, Some("charge refused")).await?;
            self.db.note_auto_reload_outcome(order.user_id, &order.currency, false).await?;
            return Ok(None);
        }
        self.reconcile(&created.id).await
    }
}
