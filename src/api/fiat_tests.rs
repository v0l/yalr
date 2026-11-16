use super::tenant_tests::{tenancy, tenancy_with_revolut, Tenancy};
use axum::body::Body;
use axum::http::Request;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use tower::util::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SECRET: &str = "whsec_test";

fn order(id: &str, state: &str, amount: u64, currency: &str, customer: bool) -> Value {
    let mut body = json!({
        "id": id, "token": format!("tok_{id}"), "state": state,
        "created_at": "2026-11-16T10:00:00.000000Z", "updated_at": "2026-11-16T10:00:00.000000Z",
        "amount": amount, "currency": currency, "outstanding_amount": amount,
    });
    if customer {
        body["customer"] = json!({"id": "cust_1"});
    }
    body
}

async fn mount(mock: &MockServer, verb: &str, route: &str, body: Value) {
    Mock::given(method(verb)).and(path(route)).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(mock).await;
}

async fn alice_id(t: &Tenancy) -> i64 {
    t.state.db.get_user_by_username("alice").await.unwrap().unwrap().id
}

async fn signed_webhook(t: &Tenancy, secret: &str, event: &str, order_id: &str) -> u16 {
    let body = format!(r#"{{"event":"{event}","order_id":"{order_id}"}}"#);
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v1.{ts}.{body}").as_bytes());
    let sig = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
    let request = Request::builder()
        .method("POST")
        .uri("/api/webhooks/revolut")
        .header("revolut-signature", sig)
        .header("revolut-request-timestamp", ts)
        .body(Body::from(body))
        .unwrap();
    t.app.clone().oneshot(request).await.unwrap().status().as_u16()
}

async fn topup(t: &Tenancy, body: &str) -> (u16, Value) {
    t.call(&t.alice, "POST", "/api/account/topup/fiat", body).await
}

#[tokio::test]
async fn card_topup_routes_are_absent_without_the_revolut_section() {
    let t = tenancy().await;
    assert!([404, 405].contains(&topup(&t, r#"{"currency":"eur","amount":10}"#).await.0));
    assert_eq!(t.get(&t.alice, "/api/account/payment-methods").await.0, 404);
    assert_eq!(t.get(&t.alice, "/api/account/topup/fiat/config").await.0, 404);
    assert_eq!(t.get(&t.admin, "/api/payments/fiat-orders").await.0, 404);
    assert!([404, 405].contains(&signed_webhook(&t, SECRET, "ORDER_COMPLETED", "x").await));
}

#[tokio::test]
async fn creating_a_topup_returns_the_widget_token_and_credits_nothing_yet() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let (status, body) = topup(&t, r#"{"currency":"EUR","amount":10}"#).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["token"], "tok_rev_1");
    assert_eq!(body["mode"], "sandbox");
    assert_eq!(body["amount_milli"], 1_000_000);
    let row = t.state.db.fiat_order_by_external("rev_1").await.unwrap().unwrap();
    assert_eq!((row.status.as_str(), row.currency.as_str()), ("pending", "eur"));
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn currency_and_amount_limits_are_enforced_before_any_order_is_created() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    for body in [
        r#"{"currency":"gbp","amount":10}"#,
        r#"{"currency":"eur","amount":4.99}"#,
        r#"{"currency":"eur","amount":500.01}"#,
        r#"{"currency":"eur","amount":10.005}"#,
        r#"{"currency":"eur","amount":-10}"#,
    ] {
        assert_eq!(topup(&t, body).await.0, 400, "{body}");
    }
    assert_eq!(topup(&t, r#"{"currency":"eur","amount":500}"#).await.0, 200);
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_webhook_with_a_bad_signature_is_refused_and_credits_nothing() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 1000, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    assert_eq!(signed_webhook(&t, "wrong_secret", "ORDER_COMPLETED", "rev_1").await, 401);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn two_webhooks_and_a_poll_credit_the_amount_paid_once() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1050, "EUR", false)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 1050, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let (_, created) = topup(&t, r#"{"currency":"eur","amount":10.5}"#).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(signed_webhook(&t, SECRET, "ORDER_COMPLETED", "rev_1").await, 200);
    assert_eq!(signed_webhook(&t, SECRET, "ORDER_COMPLETED", "rev_1").await, 200);
    let (status, polled) = t.get(&t.alice, &format!("/api/account/topup/fiat/{id}")).await;
    assert_eq!((status, polled["status"].as_str()), (200, Some("completed")));

    let user = alice_id(&t).await;
    assert_eq!(t.state.db.get_currency_balance(user, "eur").await.unwrap(), 1_050_000);
    let ledger = t.state.db.list_transactions_for_reference(&format!("fiat-{id}")).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!((ledger[0].currency.as_str(), ledger[0].amount_milli), ("eur", 1_050_000));
}

#[tokio::test]
async fn the_reconciliation_poll_alone_settles_an_order_whose_webhook_never_arrived() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 2000, "USD", false)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 2000, "USD", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    topup(&t, r#"{"currency":"usd","amount":20}"#).await;
    t.state.db.execute_raw("UPDATE fiat_orders SET created_at = UTC_TIMESTAMP(3) - INTERVAL 1 HOUR").await.unwrap();
    let service = t.state.payments_state.as_ref().unwrap().revolut.clone().unwrap();
    assert_eq!(service.reconcile_stale().await, 1);
    assert_eq!(service.reconcile_stale().await, 0);
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "usd").await.unwrap(), 2_000_000);
}

#[tokio::test]
async fn failed_and_cancelled_orders_credit_nothing() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "failed", 1000, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    assert_eq!(signed_webhook(&t, SECRET, "ORDER_FAILED", "rev_1").await, 200);
    assert_eq!(signed_webhook(&t, SECRET, "ORDER_COMPLETED", "rev_1").await, 200);
    let row = t.state.db.fiat_order_by_external("rev_1").await.unwrap().unwrap();
    assert_eq!(row.status, "failed");
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn a_completed_order_for_a_different_amount_is_not_credited() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 99999, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    signed_webhook(&t, SECRET, "ORDER_COMPLETED", "rev_1").await;
    assert_eq!(t.state.db.get_currency_balance(alice_id(&t).await, "eur").await.unwrap(), 0);
}

#[tokio::test]
async fn orders_are_private_to_their_owner() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", false)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let (_, created) = topup(&t, r#"{"currency":"eur","amount":10}"#).await;
    let uri = format!("/api/account/topup/fiat/{}", created["id"].as_str().unwrap());
    assert_eq!(t.get(&t.bob, &uri).await.0, 404);
    assert_eq!(t.get(&t.bob, "/api/account/topup/fiat").await.1, json!([]));
    assert_eq!(t.get(&t.alice, "/api/payments/fiat-orders").await.0, 403);
    let (status, all) = t.get(&t.admin, "/api/payments/fiat-orders").await;
    assert_eq!((status, all.as_array().map(Vec::len)), (200, Some(1)));
}

async fn save_a_card(t: &Tenancy, mock: &MockServer) -> i64 {
    let (_, created) = topup(t, r#"{"currency":"eur","amount":10,"save_card":true,"email":"a@example.com"}"#).await;
    assert!(created["token"].is_string(), "{created}");
    mount(mock, "GET", "/api/1.0/customers/cust_1/payment-methods", json!([
        {"id": "pm_1", "type": "CARD", "saved_for": "MERCHANT",
         "method_details": {"brand": "VISA", "last4": "4242", "expiry_month": 12, "expiry_year": 2030}}
    ])).await;
    assert_eq!(signed_webhook(t, SECRET, "ORDER_COMPLETED", "rev_1").await, 200);
    alice_id(t).await
}

#[tokio::test]
async fn a_saved_card_is_stored_with_an_encrypted_customer_and_can_be_removed_at_revolut() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", true)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 1000, "EUR", true)).await;
    Mock::given(method("DELETE")).and(path("/api/customers/cust_1/payment-methods/pm_1")).respond_with(ResponseTemplate::new(204)).expect(1).mount(&mock).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let user = save_a_card(&t, &mock).await;

    let stored = t.state.db.list_payment_methods(user).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert!(!stored[0].external_customer_id.contains("cust_1"));
    let (_, cards) = t.get(&t.alice, "/api/account/payment-methods").await;
    assert_eq!(cards[0]["last_four"], "4242");
    assert_eq!(cards[0]["is_default"], true);
    assert!(!cards.to_string().contains("cust_1"));
    assert_eq!(t.get(&t.bob, "/api/account/payment-methods").await.1, json!([]));

    let uri = format!("/api/account/payment-methods/{}", stored[0].id);
    assert_eq!(t.call(&t.bob, "DELETE", &uri, "").await.0, 404);
    assert_eq!(t.call(&t.alice, "DELETE", &uri, "").await.0, 204);
    assert!(t.state.db.list_payment_methods(user).await.unwrap().is_empty());
}

#[tokio::test]
async fn auto_reload_charges_a_low_balance_once_off_session_and_credits_on_completion() {
    let mock = MockServer::start().await;
    mount(&mock, "POST", "/api/orders", order("rev_1", "pending", 1000, "EUR", true)).await;
    mount(&mock, "GET", "/api/orders/rev_1", order("rev_1", "completed", 1000, "EUR", true)).await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let user = save_a_card(&t, &mock).await;
    let card = t.state.db.list_payment_methods(user).await.unwrap()[0].id;
    let put = format!(r#"{{"currency":"eur","enabled":true,"threshold":20,"amount":15,"monthly_cap":100,"payment_method_id":{card}}}"#);
    assert_eq!(t.call(&t.alice, "PUT", "/api/account/auto-reload", &put).await.0, 200);

    mock.reset().await;
    mount(&mock, "POST", "/api/orders", order("rev_auto", "pending", 1500, "EUR", true)).await;
    mount(&mock, "POST", "/api/orders/rev_auto/payments", json!({"id": "pay_1", "state": "pending", "amount": 1500})).await;
    mount(&mock, "GET", "/api/orders/rev_auto", order("rev_auto", "completed", 1500, "EUR", true)).await;
    let service = t.state.payments_state.as_ref().unwrap().revolut.clone().unwrap();
    let (a, b) = tokio::join!(service.run_auto_reloads(), service.run_auto_reloads());
    assert_eq!(a + b, 1);
    assert_eq!(service.run_auto_reloads().await, 0);

    let paid = mock.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/api/orders/rev_auto/payments").count();
    assert_eq!(paid, 1);
    assert_eq!(t.state.db.get_currency_balance(user, "eur").await.unwrap(), 1_000_000 + 1_500_000);
}

#[tokio::test]
async fn auto_reload_settings_reject_unknown_cards_and_bad_amounts() {
    let mock = MockServer::start().await;
    let t = tenancy_with_revolut(&mock.uri()).await;
    let body = r#"{"currency":"eur","enabled":true,"threshold":20,"amount":15,"monthly_cap":100,"payment_method_id":999}"#;
    assert_eq!(t.call(&t.alice, "PUT", "/api/account/auto-reload", body).await.0, 404);
    let capped = r#"{"currency":"eur","enabled":true,"threshold":20,"amount":15,"monthly_cap":10,"payment_method_id":1}"#;
    assert_eq!(t.call(&t.alice, "PUT", "/api/account/auto-reload", capped).await.0, 400);
}
