use super::{checkout_url, expires, remote_amount, remote_currency, GatewayError, GatewayService, RemoteSavedMethod};
use crate::db::auto_reload::AutoReload;
use crate::db::gateway_payments::{GatewayPayment, RemotePayment};
use payment_gateway_client::CreatePayment;

impl GatewayService {
    pub async fn user_cards(&self, user_id: i64) -> Result<Vec<RemoteSavedMethod>, GatewayError> {
        Ok(self.client.list_saved_methods(Some(&Self::customer_ref(user_id))).collect().await?)
    }

    pub async fn remove_card(&self, user_id: i64, id: i64) -> Result<bool, GatewayError> {
        if !self.user_cards(user_id).await?.iter().any(|c| c.id as i64 == id) {
            return Ok(false);
        }
        self.client.delete_saved_method(id as u64).await?;
        Ok(true)
    }

    pub async fn charge_auto_reload(&self, order: &GatewayPayment, settings: &AutoReload) -> Result<Option<GatewayPayment>, GatewayError> {
        if !self.user_cards(order.user_id).await?.iter().any(|c| c.id as i64 == settings.payment_method_id) {
            self.db.fail_unsent_gateway_payment(order.id, "saved card no longer exists").await?;
            self.db.note_auto_reload_outcome(order.user_id, &order.currency, false).await?;
            return Ok(None);
        }
        let body = CreatePayment {
            reference: Some(order.public_id.clone()),
            amount: remote_amount(&order.currency, order.amount_milli) as u64,
            currency: remote_currency(&order.currency),
            expires_in: Some(3600),
            customer_ref: Some(Self::customer_ref(order.user_id)),
            description: Some(format!("YALR balance auto-reload {}", order.public_id)),
            methods: Some(vec!["revolut".to_string()]),
            ..Default::default()
        };
        let charged = match self.client.charge_saved_method_with_key(settings.payment_method_id as u64, &body, &order.public_id).await {
            Ok(view) => view,
            Err(e) => {
                self.db.fail_unsent_gateway_payment(order.id, "gateway refused the charge").await?;
                self.db.note_auto_reload_outcome(order.user_id, &order.currency, false).await?;
                return Err(e.into());
            }
        };
        let url = checkout_url(&charged).or(Some(charged.pay_url.clone()));
        self.db
            .attach_remote_payment(
                order.id,
                RemotePayment { gateway_payment_id: &charged.id, pay_url: url.as_deref(), bolt11: None, payment_hash: None, expires_at: expires(&charged) },
            )
            .await?;
        self.apply(&charged).await
    }
}
