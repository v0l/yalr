use super::fiat_orders::{FiatOrder, KIND_AUTO_RELOAD};
use super::{Database, Timestamp};

pub const MAX_CONSECUTIVE_FAILURES: i32 = 2;
const MILLI_PER_MINOR: i64 = 1_000;

const COLUMNS: &str = "id, user_id, currency, enabled, threshold_milli, amount_milli, monthly_cap_milli, payment_method_id, \
    consecutive_failures, last_attempt_at, last_failure_at, disabled_reason";

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct AutoReload {
    pub id: i64,
    pub user_id: i64,
    pub currency: String,
    pub enabled: bool,
    pub threshold_milli: i64,
    pub amount_milli: i64,
    pub monthly_cap_milli: i64,
    pub payment_method_id: i64,
    pub consecutive_failures: i32,
    pub last_attempt_at: Option<Timestamp>,
    pub last_failure_at: Option<Timestamp>,
    pub disabled_reason: Option<String>,
}

pub struct AutoReloadSettings<'a> {
    pub user_id: i64,
    pub currency: &'a str,
    pub enabled: bool,
    pub threshold_milli: i64,
    pub amount_milli: i64,
    pub monthly_cap_milli: i64,
    pub payment_method_id: i64,
}

impl Database {
    pub async fn save_auto_reload(&self, s: AutoReloadSettings<'_>) -> Result<AutoReload, sqlx::Error> {
        sqlx::query(
            "INSERT INTO auto_reload_settings \
             (user_id, currency, enabled, threshold_milli, amount_milli, monthly_cap_milli, payment_method_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?) ON DUPLICATE KEY UPDATE \
             consecutive_failures = IF(VALUES(enabled) = 1 AND enabled = 0, 0, consecutive_failures), \
             disabled_reason = IF(VALUES(enabled) = 1, NULL, disabled_reason), \
             enabled = VALUES(enabled), threshold_milli = VALUES(threshold_milli), amount_milli = VALUES(amount_milli), \
             monthly_cap_milli = VALUES(monthly_cap_milli), payment_method_id = VALUES(payment_method_id)",
        )
        .bind(s.user_id)
        .bind(s.currency)
        .bind(s.enabled)
        .bind(s.threshold_milli)
        .bind(s.amount_milli)
        .bind(s.monthly_cap_milli)
        .bind(s.payment_method_id)
        .execute(&self.pool)
        .await?;
        Ok(self.auto_reload(s.user_id, s.currency).await?.expect("settings just saved"))
    }

    pub async fn auto_reload(&self, user_id: i64, currency: &str) -> Result<Option<AutoReload>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM auto_reload_settings WHERE user_id = ? AND currency = ?"))
            .bind(user_id)
            .bind(currency)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_auto_reloads(&self, user_id: i64) -> Result<Vec<AutoReload>, sqlx::Error> {
        sqlx::query_as(&format!("SELECT {COLUMNS} FROM auto_reload_settings WHERE user_id = ? ORDER BY currency"))
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn auto_reloads_below_threshold(&self) -> Result<Vec<AutoReload>, sqlx::Error> {
        sqlx::query_as(&format!(
            "SELECT {} FROM auto_reload_settings a WHERE a.enabled = 1 AND COALESCE(\
             (SELECT balance_milli FROM user_currency_balances b WHERE b.user_id = a.user_id AND b.currency = a.currency), 0) \
             < a.threshold_milli",
            COLUMNS.split(", ").map(|c| format!("a.{c}")).collect::<Vec<_>>().join(", ")
        ))
        .fetch_all(&self.pool)
        .await
    }

    pub async fn claim_auto_reload(
        &self,
        user_id: i64,
        currency: &str,
        cooldown_secs: i64,
    ) -> Result<Option<(FiatOrder, AutoReload)>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let settings: Option<AutoReload> = sqlx::query_as(&format!(
            "SELECT {COLUMNS} FROM auto_reload_settings WHERE user_id = ? AND currency = ? AND enabled = 1 FOR UPDATE"
        ))
        .bind(user_id)
        .bind(currency)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(settings) = settings else {
            tx.rollback().await?;
            return Ok(None);
        };
        let balance: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT balance_milli FROM user_currency_balances WHERE user_id = ? AND currency = ?), 0)")
            .bind(user_id)
            .bind(currency)
            .fetch_one(&mut *tx)
            .await?;
        let in_flight: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM fiat_orders WHERE user_id = ? AND currency = ? AND kind = ? AND status = 'pending'",
        )
        .bind(user_id)
        .bind(currency)
        .bind(KIND_AUTO_RELOAD)
        .fetch_one(&mut *tx)
        .await?;
        let cooling: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM auto_reload_settings WHERE id = ? AND last_failure_at IS NOT NULL \
             AND last_failure_at > UTC_TIMESTAMP(3) - INTERVAL ? SECOND",
        )
        .bind(settings.id)
        .bind(cooldown_secs * settings.consecutive_failures.max(1) as i64)
        .fetch_one(&mut *tx)
        .await?;
        let spent: i64 = sqlx::query_scalar(
            "SELECT CAST(COALESCE(SUM(amount_milli), 0) AS SIGNED) FROM fiat_orders WHERE user_id = ? AND currency = ? \
             AND kind = ? AND status IN ('pending', 'completed') \
             AND created_at >= DATE_FORMAT(UTC_TIMESTAMP(3), '%Y-%m-01')",
        )
        .bind(user_id)
        .bind(currency)
        .bind(KIND_AUTO_RELOAD)
        .fetch_one(&mut *tx)
        .await?;
        let wanted = settings.amount_milli.max(settings.threshold_milli - balance);
        let wanted = (wanted + MILLI_PER_MINOR - 1) / MILLI_PER_MINOR * MILLI_PER_MINOR;
        let room = (settings.monthly_cap_milli - spent) / MILLI_PER_MINOR * MILLI_PER_MINOR;
        let charge = wanted.min(room);
        if balance >= settings.threshold_milli
            || in_flight > 0
            || cooling > 0
            || charge < settings.amount_milli
        {
            tx.rollback().await?;
            return Ok(None);
        }
        let public_id = super::random_hex();
        sqlx::query(
            "INSERT INTO fiat_orders (public_id, user_id, kind, currency, amount_milli, save_card) VALUES (?, ?, ?, ?, ?, 0)",
        )
        .bind(&public_id)
        .bind(user_id)
        .bind(KIND_AUTO_RELOAD)
        .bind(currency)
        .bind(charge)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE auto_reload_settings SET last_attempt_at = UTC_TIMESTAMP(3) WHERE id = ?")
            .bind(settings.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let order = self.fiat_order(&public_id).await?.expect("order just inserted");
        Ok(Some((order, settings)))
    }

    pub async fn note_auto_reload_outcome(&self, user_id: i64, currency: &str, succeeded: bool) -> Result<(), sqlx::Error> {
        if succeeded {
            sqlx::query("UPDATE auto_reload_settings SET consecutive_failures = 0, last_failure_at = NULL WHERE user_id = ? AND currency = ?")
                .bind(user_id)
                .bind(currency)
                .execute(&self.pool)
                .await?;
            return Ok(());
        }
        sqlx::query(
            "UPDATE auto_reload_settings SET \
             enabled = IF(consecutive_failures + 1 >= ?, 0, enabled), \
             disabled_reason = IF(consecutive_failures + 1 >= ?, 'Disabled after repeated payment failures', disabled_reason), \
             consecutive_failures = consecutive_failures + 1, \
             last_failure_at = UTC_TIMESTAMP(3) \
             WHERE user_id = ? AND currency = ?",
        )
        .bind(MAX_CONSECUTIVE_FAILURES)
        .bind(MAX_CONSECUTIVE_FAILURES)
        .bind(user_id)
        .bind(currency)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::payment_methods::NewPaymentMethod;

    struct Fixture {
        db: Database,
        user: i64,
    }

    async fn fixture(cap_units: i64) -> Fixture {
        let db = Database::test().await;
        let user = db.add_test_user("u", false).await;
        let pm = db
            .add_payment_method(NewPaymentMethod {
                user_id: user.id,
                external_customer_id_encrypted: "sealed",
                external_id: "pm_1",
                card_brand: None,
                card_last_four: None,
                exp_month: None,
                exp_year: None,
            })
            .await
            .unwrap();
        db.save_auto_reload(AutoReloadSettings {
            user_id: user.id,
            currency: "eur",
            enabled: true,
            threshold_milli: 500_000,
            amount_milli: 2_000_000,
            monthly_cap_milli: cap_units * 100_000,
            payment_method_id: pm.id,
        })
        .await
        .unwrap();
        Fixture { db, user: user.id }
    }

    #[tokio::test]
    async fn only_balances_under_the_threshold_are_listed() {
        let f = fixture(1000).await;
        assert_eq!(f.db.auto_reloads_below_threshold().await.unwrap().len(), 1);
        f.db.credit_currency(f.user, "eur", 500_000, "admin_credit", "c", 0).await.unwrap();
        assert!(f.db.auto_reloads_below_threshold().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn many_concurrent_claims_create_one_order() {
        let f = fixture(1000).await;
        let claims: Vec<_> = (0..10)
            .map(|_| {
                let db = f.db.clone();
                let user = f.user;
                tokio::spawn(async move { db.claim_auto_reload(user, "eur", 3600).await.unwrap().is_some() })
            })
            .collect();
        let mut won = 0;
        for c in claims {
            won += c.await.unwrap() as i32;
        }
        assert_eq!(won, 1);
        assert!(f.db.claim_auto_reload(f.user, "eur", 3600).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_failure_cools_down_then_a_second_failure_disables() {
        let f = fixture(1000).await;
        let (first, _) = f.db.claim_auto_reload(f.user, "eur", 3600).await.unwrap().unwrap();
        f.db.fail_unsent_fiat_order(first.id, "declined").await.unwrap();
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        assert!(f.db.claim_auto_reload(f.user, "eur", 3600).await.unwrap().is_none());
        assert!(f.db.auto_reload(f.user, "eur").await.unwrap().unwrap().enabled);

        let (second, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
        f.db.fail_unsent_fiat_order(second.id, "declined").await.unwrap();
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        let settings = f.db.auto_reload(f.user, "eur").await.unwrap().unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.disabled_reason.as_deref(), Some("Disabled after repeated payment failures"));
        assert!(f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn success_resets_the_failure_count() {
        let f = fixture(1000).await;
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        f.db.note_auto_reload_outcome(f.user, "eur", true).await.unwrap();
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        assert!(f.db.auto_reload(f.user, "eur").await.unwrap().unwrap().enabled);
    }

    #[tokio::test]
    async fn the_monthly_cap_stops_further_charges() {
        let f = fixture(40).await;
        for _ in 0..1 {
            let (order, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
            f.db.set_fiat_order_external(order.id, &format!("rev_{}", order.id)).await.unwrap();
            f.db.complete_fiat_order(&format!("rev_{}", order.id)).await.unwrap();
            f.db.execute_raw("UPDATE user_currency_balances SET balance_milli = 0").await.unwrap();
        }
        let (order, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
        f.db.set_fiat_order_external(order.id, "rev_second").await.unwrap();
        f.db.complete_fiat_order("rev_second").await.unwrap();
        f.db.execute_raw("UPDATE user_currency_balances SET balance_milli = 0").await.unwrap();
        assert!(f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().is_none());
    }

    async fn configure(f: &Fixture, monthly_cap_milli: i64) {
        let pm = f.db.list_payment_methods(f.user).await.unwrap()[0].id;
        f.db.save_auto_reload(AutoReloadSettings {
            user_id: f.user,
            currency: "eur",
            enabled: true,
            threshold_milli: 5_000_000,
            amount_milli: 1_500_000,
            monthly_cap_milli,
            payment_method_id: pm,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn one_trigger_makes_one_charge_sized_to_reach_the_threshold() {
        let f = fixture(1000).await;
        configure(&f, 100_000_000).await;
        f.db.credit_currency(f.user, "eur", 1_000_000, "admin_credit", "c", 0).await.unwrap();

        let (order, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
        assert_eq!(order.amount_milli, 4_000_000);
        assert!(f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().is_none());

        f.db.set_fiat_order_external(order.id, "rev_sized").await.unwrap();
        f.db.complete_fiat_order("rev_sized").await.unwrap();
        assert_eq!(f.db.get_currency_balance(f.user, "eur").await.unwrap(), 5_000_000);
        assert!(f.db.auto_reloads_below_threshold().await.unwrap().is_empty());
        assert!(f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_reload_amount_is_the_floor_when_the_shortfall_is_smaller() {
        let f = fixture(1000).await;
        configure(&f, 100_000_000).await;
        f.db.credit_currency(f.user, "eur", 4_800_000, "admin_credit", "c", 0).await.unwrap();
        let (order, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
        assert_eq!(order.amount_milli, 1_500_000);
    }

    #[tokio::test]
    async fn the_sized_charge_is_trimmed_to_the_remaining_monthly_cap() {
        let f = fixture(1000).await;
        configure(&f, 3_000_000).await;
        let (order, _) = f.db.claim_auto_reload(f.user, "eur", 0).await.unwrap().unwrap();
        assert_eq!(order.amount_milli, 3_000_000);
    }

    #[tokio::test]
    async fn re_enabling_clears_the_reason_and_failures() {
        let f = fixture(1000).await;
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        f.db.note_auto_reload_outcome(f.user, "eur", false).await.unwrap();
        let s = f.db.auto_reload(f.user, "eur").await.unwrap().unwrap();
        assert!(!s.enabled);
        let again = f
            .db
            .save_auto_reload(AutoReloadSettings {
                user_id: f.user,
                currency: "eur",
                enabled: true,
                threshold_milli: s.threshold_milli,
                amount_milli: s.amount_milli,
                monthly_cap_milli: s.monthly_cap_milli,
                payment_method_id: s.payment_method_id,
            })
            .await
            .unwrap();
        assert!(again.enabled && again.consecutive_failures == 0 && again.disabled_reason.is_none());
    }
}
