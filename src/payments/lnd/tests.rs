use super::fake::FakeLnd;
use super::*;
use crate::db::gateway_payments::{COMPLETED, KIND_TOPUP, PENDING};
use crate::payments::provider::PaymentProvider;
use std::time::Duration;

struct Rig {
    db: Arc<Database>,
    node: Arc<FakeLnd>,
    service: Arc<LndService>,
    user: i64,
}

async fn rig() -> Rig {
    let db = Arc::new(Database::test().await);
    let user = db.add_test_user("alice", false).await.id;
    let node = Arc::new(FakeLnd::default());
    let service = Arc::new(LndService::new(node.clone(), db.clone(), LndSettings::new("https://lnd", "cert", "mac")));
    Rig { db, node, service, user }
}

impl Rig {
    async fn invoice(&self, sats: i64) -> GatewayPayment {
        self.service
            .create_topup(TopupRequest {
                user_id: self.user,
                kind: KIND_TOPUP,
                purpose: None,
                currency: SATS,
                amount_milli: sats * 1000,
                lightning: true,
                email: None,
                public_id: None,
            })
            .await
            .unwrap()
    }

    async fn balance(&self) -> i64 {
        self.db.get_currency_balance(self.user, SATS).await.unwrap()
    }

    async fn status(&self, p: &GatewayPayment) -> String {
        self.db.gateway_payment(&p.public_id).await.unwrap().unwrap().status
    }

    async fn cursor(&self) -> u64 {
        self.db.payment_cursor(SETTLE_CURSOR).await.unwrap()
    }
}

async fn eventually<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn an_invoice_is_stored_with_its_bolt11_hash_and_expiry() {
    let r = rig().await;
    let p = r.invoice(2500).await;
    assert_eq!(p.status, PENDING);
    assert_eq!(p.amount_milli, 2_500_000);
    assert_eq!(p.bolt11.as_deref(), Some("lnbcrt2500fake1"));
    assert_eq!(p.payment_hash.as_deref(), Some(r.node.hash_of(1).as_str()));
    assert_eq!(p.gateway_payment_id, p.payment_hash);
    assert!(p.pay_url.is_none());
    let left = (p.expires_at.unwrap() - chrono::Utc::now()).num_seconds();
    assert!((3590..=3600).contains(&left), "{left}");
}

#[tokio::test]
async fn a_card_currency_is_refused_and_says_the_gateway_is_needed() {
    let r = rig().await;
    let err = r.service.check_amount("eur", 10.0).unwrap_err().to_string();
    assert!(err.contains("payments.gateway"), "{err}");
    assert_eq!(r.service.currencies(), vec!["sats"]);
    assert_eq!(r.service.mode(), "mainnet");
}

#[tokio::test]
async fn when_lnd_cannot_make_the_invoice_the_row_fails_and_nothing_is_owed() {
    let r = rig().await;
    r.node.state.lock().unwrap().fail_add = true;
    let err = r.service.create_topup(TopupRequest { user_id: r.user, kind: KIND_TOPUP, purpose: None, currency: SATS, amount_milli: 1_000_000, lightning: true, email: None, public_id: Some("pid") }).await;
    assert!(matches!(err, Err(PaymentError::Upstream(_))));
    assert_eq!(r.db.gateway_payment("pid").await.unwrap().unwrap().status, "failed");
}

#[tokio::test]
async fn the_stream_credits_a_settled_invoice_once_and_stores_the_cursor() {
    let r = rig().await;
    let p = r.invoice(1000).await;
    spawn_follower(r.service.clone(), Duration::from_millis(10));
    eventually(|| async { !r.node.subscriptions().is_empty() }).await;
    let settled = r.node.settle(p.payment_hash.as_deref().unwrap());
    eventually(|| async { r.cursor().await == 1 }).await;
    r.node.replay(&settled);
    r.node.replay(&settled);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.status(&p).await, COMPLETED);
    assert_eq!(r.balance().await, 1_000_000);
    assert_eq!(r.db.list_transactions_for_reference(&format!("gw-{}", p.public_id)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_restart_resumes_from_the_stored_settle_index() {
    let r = rig().await;
    let first = r.invoice(1000).await;
    let second = r.invoice(2000).await;
    r.node.settle_while_offline(first.payment_hash.as_deref().unwrap());
    r.db.advance_payment_cursor(SETTLE_CURSOR, 1).await.unwrap();
    r.service.apply(&r.node.lookup_invoice(first.payment_hash.as_deref().unwrap()).await.unwrap().unwrap()).await.unwrap();
    r.node.settle_while_offline(second.payment_hash.as_deref().unwrap());

    spawn_follower(r.service.clone(), Duration::from_millis(10));
    eventually(|| async { r.cursor().await == 2 }).await;
    assert_eq!(r.node.subscriptions()[0], 1);
    assert_eq!(r.status(&second).await, COMPLETED);
    assert_eq!(r.balance().await, 3_000_000);
}

#[tokio::test]
async fn the_stream_is_resubscribed_from_the_cursor_when_it_ends() {
    let r = rig().await;
    let a = r.invoice(1000).await;
    let b = r.invoice(1000).await;
    spawn_follower(r.service.clone(), Duration::from_millis(10));
    eventually(|| async { r.node.subscriptions().len() == 1 }).await;
    r.node.settle(a.payment_hash.as_deref().unwrap());
    eventually(|| async { r.cursor().await == 1 }).await;

    r.node.drop_streams();
    r.node.settle_while_offline(b.payment_hash.as_deref().unwrap());
    eventually(|| async { r.cursor().await == 2 }).await;
    assert_eq!(r.node.subscriptions(), vec![0, 1]);
    assert_eq!(r.balance().await, 2_000_000);
}

#[tokio::test]
async fn repair_on_start_credits_invoices_settled_while_yalr_was_down_even_with_a_lost_cursor() {
    let r = rig().await;
    let paid = r.invoice(1500).await;
    let unpaid = r.invoice(700).await;
    r.node.settle_while_offline(paid.payment_hash.as_deref().unwrap());
    r.db.advance_payment_cursor(SETTLE_CURSOR, 99).await.unwrap();

    assert_eq!(r.service.repair().await, 1);
    assert_eq!(r.service.repair().await, 0);
    assert_eq!(r.status(&paid).await, COMPLETED);
    assert_eq!(r.status(&unpaid).await, PENDING);
    assert_eq!(r.balance().await, 1_500_000);
}

#[tokio::test]
async fn a_cancelled_or_lapsed_invoice_credits_nothing() {
    let r = rig().await;
    let cancelled = r.invoice(1000).await;
    let lapsed = r.invoice(1000).await;
    r.node.cancel(cancelled.payment_hash.as_deref().unwrap());
    r.db.execute_raw(&format!("UPDATE gateway_payments SET expires_at = UTC_TIMESTAMP(3) - INTERVAL 1 MINUTE WHERE id = {}", lapsed.id)).await.unwrap();
    assert_eq!(r.service.repair().await, 2);
    assert_eq!(r.status(&cancelled).await, "expired");
    assert_eq!(r.status(&lapsed).await, "expired");
    assert_eq!(r.balance().await, 0);
}

#[tokio::test]
async fn an_underpaid_settle_is_not_credited() {
    let r = rig().await;
    let p = r.invoice(1000).await;
    let event = r.node.settle_for(p.payment_hash.as_deref().unwrap(), 999_000);
    r.service.apply(&event).await.unwrap();
    assert_eq!(r.status(&p).await, "failed");
    assert_eq!(r.balance().await, 0);
}

#[tokio::test]
async fn settles_for_invoices_yalr_did_not_make_are_ignored_but_move_the_cursor() {
    let r = rig().await;
    let foreign = r.node.add_invoice(5_000_000, "someone else", 60).await.unwrap();
    spawn_follower(r.service.clone(), Duration::from_millis(10));
    eventually(|| async { !r.node.subscriptions().is_empty() }).await;
    r.node.settle(&foreign.payment_hash);
    eventually(|| async { r.cursor().await == 1 }).await;
    assert_eq!(r.balance().await, 0);
}

#[tokio::test]
async fn refunds_and_payouts_pay_through_the_same_node() {
    let r = rig().await;
    assert_eq!(r.service.refund(r.user, 1000, "lnbc1refund").await.unwrap(), "paid_1");
    let payer: Arc<dyn crate::payouts::lightning::InvoicePayer> = r.service.clone();
    assert_eq!(payer.pay("lnbc1payout").await.unwrap().payment_hash, "paid_2");
    r.node.state.lock().unwrap().fail_pay = true;
    assert!(r.service.refund(r.user, 1000, "lnbc1x").await.is_err());
}
