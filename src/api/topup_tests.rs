use super::tenant_tests::{tenancy, tenancy_with_gateway, Tenancy};
use crate::payments::gateway::fake::{FakeGateway, WEBHOOK_SECRET};
use axum::body::Body;
use axum::http::Request;
use serde_json::Value;
use tower::util::ServiceExt;

async fn alice_id(t: &Tenancy) -> i64 {
    t.state.db.get_user_by_username("alice").await.unwrap().unwrap().id
}

async fn deliver(t: &Tenancy, header: &str, body: &str) -> u16 {
    let request = Request::builder()
        .method("POST")
        .uri("/api/webhooks/gateway")
        .header("x-gateway-signature", header)
        .body(Body::from(body.to_string()))
        .unwrap();
    t.app.clone().oneshot(request).await.unwrap().status().as_u16()
}

async fn paid_webhook(t: &Tenancy, fake: &FakeGateway, id: &str) -> u16 {
    fake.set_status(id, "paid");
    let (header, body) = fake.event("payment.paid", id);
    deliver(t, &header, &body).await
}

async fn topup(t: &Tenancy, body: &str) -> (u16, Value) {
    t.call(&t.alice, "POST", "/api/account/topup", body).await
}

async fn setup() -> (FakeGateway, Tenancy) {
    let fake = FakeGateway::start().await;
    let t = tenancy_with_gateway(&fake.url).await;
    (fake, t)
}

#[tokio::test]
async fn topup_routes_are_absent_without_the_gateway_section() {
    let t = tenancy().await;
    assert!([404, 405].contains(&topup(&t, r#"{"currency":"eur","amount":10}"#).await.0));
    assert_eq!(t.get(&t.alice, "/api/account/payment-methods").await.0, 404);
    assert_eq!(t.get(&t.alice, "/api/account/topup/config").await.0, 404);
    assert_eq!(t.get(&t.admin, "/api/payments/topups").await.0, 404);
    assert!([404, 405].contains(&deliver(&t, "t=1,v1=00", "{}").await));
}

#[tokio::test]
async fn a_card_topup_returns_the_checkout_url_and_credits_nothing_yet() {
    let (fake, t) = setup().await;
    let (status, body) = topup(&t, r#"{"currency":"EUR","amount":10}"#).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "pending");
    assert_eq!(body["currency"], "eur");
    assert_eq!(body["amount_milli"], 1_000_000);
    assert!(body["pay_url"].as_str().unwrap().starts_with("https://checkout.example/"));
    let sent = fake.payment("gw1");
    assert_eq!((sent.amount, sent.currency.as_str(), sent.method.as_deref()), (1000, "EUR", Some("revolut")));
    assert_eq!(sent.customer_ref, format!("user-{}", alice_id(&t).await));
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn a_lightning_topup_asks_the_gateway_for_btc_msats_and_returns_the_invoice() {
    let (fake, t) = setup().await;
    let (status, body) = topup(&t, r#"{"currency":"sats","amount":2500}"#).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["bolt11"], "lnbcrtgw1");
    let sent = fake.payment("gw1");
    assert_eq!((sent.amount, sent.currency.as_str(), sent.method.as_deref()), (2_500_000, "BTC", Some("lightning")));
}

#[tokio::test]
async fn currency_and_amount_limits_are_enforced_before_any_payment_is_created() {
    let (fake, t) = setup().await;
    for body in [
        r#"{"currency":"gbp","amount":10}"#,
        r#"{"currency":"eur","amount":4.99}"#,
        r#"{"currency":"eur","amount":500.01}"#,
        r#"{"currency":"eur","amount":10.005}"#,
        r#"{"currency":"eur","amount":-10}"#,
        r#"{"currency":"sats","amount":0.5}"#,
        r#"{"currency":"eur","amount":10,"save_card":true}"#,
    ] {
        assert_eq!(topup(&t, body).await.0, 400, "{body}");
    }
    assert!(fake.state.lock().unwrap().requests.is_empty());
    assert_eq!(topup(&t, r#"{"currency":"eur","amount":500}"#).await.0, 200);
}

#[tokio::test]
async fn a_signed_paid_webhook_credits_the_balance_once_and_a_replay_adds_nothing() {
    let (fake, t) = setup().await;
    let (_, created) = topup(&t, r#"{"currency":"eur","amount":10.5}"#).await;
    let public_id = created["public_id"].as_str().unwrap().to_string();

    assert_eq!(paid_webhook(&t, &fake, "gw1").await, 200);
    assert_eq!(paid_webhook(&t, &fake, "gw1").await, 200);
    let (header, body) = fake.event("payment.paid", "gw1");
    assert_eq!(deliver(&t, &header, &body).await, 200);

    let user = alice_id(&t).await;
    assert_eq!(t.state.db.get_currency_balance(user, "eur").await.unwrap(), 1_050_000);
    let ledger = t.state.db.list_transactions_for_reference(&format!("gw-{public_id}")).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!((ledger[0].currency.as_str(), ledger[0].amount_milli), ("eur", 1_050_000));
    let (_, polled) = t.get(&t.alice, &format!("/api/account/topup/{public_id}")).await;
    assert_eq!(polled["status"], "completed");
}

#[tokio::test]
async fn a_paid_lightning_topup_credits_sats() {
    let (fake, t) = setup().await;
    topup(&t, r#"{"currency":"sats","amount":1000}"#).await;
    assert_eq!(paid_webhook(&t, &fake, "gw1").await, 200);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "sats").await.unwrap(), 1_000_000);
}

#[tokio::test]
async fn a_webhook_with_a_bad_signature_is_refused_and_credits_nothing() {
    let (fake, t) = setup().await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    fake.set_status("gw1", "paid");
    let now = chrono::Utc::now().timestamp();
    let (header, body) = fake.event_signed_with("wrong_secret", now, "payment.paid", "gw1");
    assert_eq!(deliver(&t, &header, &body).await, 401);
    let (header, body) = fake.event_signed_with(WEBHOOK_SECRET, now - 3600, "payment.paid", "gw1");
    assert_eq!(deliver(&t, &header, &body).await, 401);
    let (header, _) = fake.event("payment.paid", "gw1");
    assert_eq!(deliver(&t, &header, "{\"tampered\":true}").await, 401);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn a_paid_event_for_a_different_amount_is_not_credited() {
    let (fake, t) = setup().await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    fake.state.lock().unwrap().payments[0].amount = 100;
    assert_eq!(paid_webhook(&t, &fake, "gw1").await, 200);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
    let row = t.state.db.gateway_payment_by_remote("gw1").await.unwrap().unwrap();
    assert_eq!((row.status.as_str(), row.failure_reason.as_deref()), ("failed", Some("amount mismatch")));
}

#[tokio::test]
async fn expired_and_unknown_payments_are_handled_quietly() {
    let (fake, t) = setup().await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    fake.set_status("gw1", "expired");
    let (header, body) = fake.event("payment.expired", "gw1");
    assert_eq!(deliver(&t, &header, &body).await, 200);
    assert_eq!(t.state.db.gateway_payment_by_remote("gw1").await.unwrap().unwrap().status, "expired");

    fake.state.lock().unwrap().payments.push(crate::payments::gateway::fake::FakePayment {
        id: "other".into(),
        amount: 5,
        currency: "EUR".into(),
        status: "paid".into(),
        reference: "x".into(),
        customer_ref: "x".into(),
        method: None,
        save_method: false,
    });
    let (header, body) = fake.event("payment.paid", "other");
    assert_eq!(deliver(&t, &header, &body).await, 200);
}

#[tokio::test]
async fn the_reconciler_alone_settles_a_payment_whose_webhook_never_arrived() {
    let (fake, t) = setup().await;
    topup(&t, r#"{"currency":"usd","amount":20}"#).await;
    fake.set_status("gw1", "paid");
    t.state.db.execute_raw("UPDATE gateway_payments SET created_at = UTC_TIMESTAMP(3) - INTERVAL 1 HOUR").await.unwrap();
    let service = t.state.payments_state.as_ref().unwrap().gateway.clone().unwrap();
    assert_eq!(service.reconcile_stale().await, 1);
    assert_eq!(service.reconcile_stale().await, 0);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "usd").await.unwrap(), 2_000_000);
}

#[tokio::test]
async fn polling_a_topup_settles_it_and_other_users_cannot_see_it() {
    let (fake, t) = setup().await;
    let (_, created) = topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    let id = created["public_id"].as_str().unwrap().to_string();
    fake.set_status("gw1", "paid");
    let (status, polled) = t.get(&t.alice, &format!("/api/account/topup/{id}")).await;
    assert_eq!((status, polled["status"].as_str()), (200, Some("completed")));
    assert_eq!(t.get(&t.bob, &format!("/api/account/topup/{id}")).await.0, 404);
    let (_, history) = t.get(&t.alice, "/api/account/topup").await;
    assert_eq!(history.as_array().unwrap().len(), 1);
    let (_, all) = t.get(&t.admin, "/api/payments/topups").await;
    assert_eq!(all[0]["username"], "alice");
    assert_eq!(t.get(&t.alice, "/api/payments/topups").await.0, 403);
}

#[tokio::test]
async fn saved_cards_come_from_the_gateway_and_only_the_owner_can_delete_one() {
    let (fake, t) = setup().await;
    let user = alice_id(&t).await;
    fake.add_saved(7, &format!("user-{user}"));
    fake.add_saved(8, "user-9999");
    let (_, cards) = t.get(&t.alice, "/api/account/payment-methods").await;
    assert_eq!(cards.as_array().unwrap().len(), 1);
    assert_eq!((cards[0]["id"].as_i64(), cards[0]["last_four"].as_str()), (Some(7), Some("4242")));
    assert_eq!(t.call(&t.alice, "DELETE", "/api/account/payment-methods/8", "").await.0, 404);
    assert_eq!(t.call(&t.alice, "DELETE", "/api/account/payment-methods/7", "").await.0, 204);
    assert_eq!(t.get(&t.alice, "/api/account/payment-methods").await.1.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn auto_reload_charges_the_saved_card_and_credits_once() {
    let (fake, t) = setup().await;
    let user = alice_id(&t).await;
    fake.add_saved(7, &format!("user-{user}"));
    let body = r#"{"currency":"eur","enabled":true,"threshold":5,"amount":20,"monthly_cap":100,"payment_method_id":7}"#;
    assert_eq!(t.call(&t.alice, "PUT", "/api/account/auto-reload", body).await.0, 200);
    let bad = body.replace(":7}", ":8}");
    assert_eq!(t.call(&t.alice, "PUT", "/api/account/auto-reload", &bad).await.0, 404);

    let service = t.state.payments_state.as_ref().unwrap().gateway.clone().unwrap();
    assert_eq!(service.run_auto_reloads().await, 1);
    assert_eq!(service.run_auto_reloads().await, 0);
    assert_eq!(t.state.db.get_currency_balance(user, "eur").await.unwrap(), 2_000_000);
}

#[tokio::test]
async fn a_declined_auto_reload_credits_nothing_and_counts_a_failure() {
    let (fake, t) = setup().await;
    let user = alice_id(&t).await;
    fake.add_saved(7, &format!("user-{user}"));
    fake.state.lock().unwrap().decline_charges = true;
    let body = r#"{"currency":"eur","enabled":true,"threshold":5,"amount":20,"monthly_cap":100,"payment_method_id":7}"#;
    t.call(&t.alice, "PUT", "/api/account/auto-reload", body).await;
    let service = t.state.payments_state.as_ref().unwrap().gateway.clone().unwrap();
    service.run_auto_reloads().await;
    assert_eq!(t.state.db.get_currency_balance(user, "eur").await.unwrap(), 0);
    assert_eq!(t.state.db.auto_reload(user, "eur").await.unwrap().unwrap().consecutive_failures, 1);
}
