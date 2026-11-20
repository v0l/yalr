use crate::api::topup::{AutoReloadView, SavedCard, TopupConfig};
use crate::db::gateway_payments::{GatewayPayment, GatewayPaymentWithUser};
use crate::payments::account::InvoiceEntry;
use chrono::{TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;

fn fixture(name: &str) -> Value {
    let path = format!("{}/admin/src/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn at(h: u32, m: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 2, h, m, 5).unwrap()
}

fn assert_matches<T: Serialize>(name: &str, value: &T) {
    assert_eq!(serde_json::to_value(value).unwrap(), fixture(name), "{name} drifted from admin/src/fixtures");
}

#[test]
fn gateway_payment_json_matches_the_admin_fixture() {
    let payment = GatewayPayment {
        id: 7,
        public_id: "pay_abc123".into(),
        gateway_payment_id: Some("gw1".into()),
        user_id: 3,
        kind: "topup".into(),
        purpose: None,
        currency: "eur".into(),
        amount_milli: 1_000_000,
        status: "failed".into(),
        bolt11: None,
        payment_hash: None,
        pay_url: Some("https://checkout.example/gw1".into()),
        failure_reason: Some("card declined".into()),
        created_at: at(3, 4),
        updated_at: at(3, 5),
        expires_at: Some(at(4, 4)),
        completed_at: None,
    };
    assert_matches("gateway_payment", &GatewayPaymentWithUser { payment, username: "alice".into() });
}

#[test]
fn account_invoice_json_matches_the_admin_fixture() {
    let entry = InvoiceEntry {
        invoice_id: "inv_abc123".into(),
        purpose: "topup".into(),
        amount_sats: 2100,
        status: "paid".into(),
        created_at: at(3, 4),
        paid_at: Some(at(3, 6)),
    };
    assert_matches("account_invoice", &entry);
}

#[test]
fn saved_card_auto_reload_and_config_json_match_the_admin_fixtures() {
    assert_matches("saved_card", &SavedCard { id: 4, brand: Some("visa".into()), last_four: Some("4242".into()), exp_month: Some(12), exp_year: Some(2030) });
    assert_matches(
        "auto_reload",
        &AutoReloadView {
            currency: "eur".into(),
            enabled: true,
            threshold_milli: 500_000,
            amount_milli: 2_000_000,
            monthly_cap_milli: 10_000_000,
            payment_method_id: 4,
            consecutive_failures: 0,
            disabled_reason: None,
        },
    );
    assert_matches("topup_config", &TopupConfig { mode: "sandbox", currencies: vec!["eur".into(), "usd".into(), "sats".into()], min_amount: 5, max_amount: 500 });
}
