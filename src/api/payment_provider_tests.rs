use super::tenant_tests::{tenancy_with_gateway, tenancy_with_lnd, Tenancy};
use crate::payments::anonymous::derive_api_key;
use crate::payments::gateway::fake::FakeGateway;
use crate::payments::lnd::fake::FakeLnd;
use serde_json::Value;
use std::sync::Arc;

enum Backend {
    Gateway(FakeGateway),
    Lnd(Arc<FakeLnd>),
}

struct Rig {
    t: Tenancy,
    backend: Backend,
}

async fn gateway() -> Rig {
    let fake = FakeGateway::start().await;
    Rig { t: tenancy_with_gateway(&fake.url).await, backend: Backend::Gateway(fake) }
}

async fn lnd() -> Rig {
    let node = Arc::new(FakeLnd::default());
    Rig { t: tenancy_with_lnd(node.clone()).await, backend: Backend::Lnd(node) }
}

impl Rig {
    async fn alice(&self) -> i64 {
        self.t.state.db.get_user_by_username("alice").await.unwrap().unwrap().id
    }

    async fn sats(&self, user: i64) -> i64 {
        self.t.state.db.get_currency_balance(user, "sats").await.unwrap()
    }

    fn remote_of(&self, payment_hash: &str) -> String {
        match &self.backend {
            Backend::Gateway(fake) => {
                let s = fake.state.lock().unwrap();
                s.payments.iter().rev().find(|p| format!("hash_{}", p.id) == payment_hash).map(|p| p.id.clone()).unwrap_or_else(|| fake.last_id())
            }
            Backend::Lnd(_) => payment_hash.to_string(),
        }
    }

    async fn settle(&self, payment_hash: &str) {
        let remote = self.remote_of(payment_hash);
        match &self.backend {
            Backend::Gateway(fake) => {
                fake.set_status(&remote, "paid");
                let (header, body) = fake.event("payment.paid", &remote);
                assert_eq!(self.webhook(&header, &body).await, 200);
            }
            Backend::Lnd(node) => {
                let event = node.settle(&remote);
                self.lnd_service().apply(&event).await.unwrap();
            }
        }
    }

    async fn replay_settle(&self, payment_hash: &str) {
        let remote = self.remote_of(payment_hash);
        match &self.backend {
            Backend::Gateway(fake) => {
                let (header, body) = fake.event("payment.paid", &remote);
                assert_eq!(self.webhook(&header, &body).await, 200);
            }
            Backend::Lnd(node) => {
                let event = node.lookup_invoice_now(&remote);
                self.lnd_service().apply(&event).await.unwrap();
            }
        }
    }

    async fn expire(&self, payment_hash: &str) {
        let remote = self.remote_of(payment_hash);
        match &self.backend {
            Backend::Gateway(fake) => {
                fake.set_status(&remote, "expired");
                let (header, body) = fake.event("payment.expired", &remote);
                assert_eq!(self.webhook(&header, &body).await, 200);
            }
            Backend::Lnd(node) => {
                let event = node.cancel(&remote);
                self.lnd_service().apply(&event).await.unwrap();
            }
        }
    }

    async fn settle_late_after_expiry(&self, payment_hash: &str) {
        let remote = self.remote_of(payment_hash);
        if let Backend::Gateway(fake) = &self.backend {
            fake.set_status(&remote, "paid");
        }
        let _ = self.t.get(&self.t.alice, "/api/account/topup").await;
    }

    fn lnd_service(&self) -> Arc<crate::payments::lnd::LndService> {
        self.t.state.payments_state.as_ref().unwrap().lnd.clone().unwrap()
    }

    async fn webhook(&self, header: &str, body: &str) -> u16 {
        use tower::util::ServiceExt;
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/webhooks/gateway")
            .header("x-gateway-signature", header)
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();
        self.t.app.clone().oneshot(request).await.unwrap().status().as_u16()
    }

    async fn lightning_invoice(&self, sats: u64) -> (u16, Value) {
        self.t.call(&self.t.alice, "POST", "/lightning/invoice", &format!(r#"{{"amount_sats":{sats}}}"#)).await
    }

    async fn anonymous_invoice(&self, sats: u64) -> (u16, Value) {
        self.t.call("none", "POST", "/v2/lightning/invoice", &format!(r#"{{"amount_sats":{sats},"purpose":"create"}}"#)).await
    }
}

impl FakeLnd {
    fn lookup_invoice_now(&self, hash: &str) -> crate::payments::lnd::InvoiceEvent {
        self.state.lock().unwrap().invoices.get(hash).map(|(_, e)| e.clone()).unwrap()
    }
}

async fn a_lightning_topup_is_created_with_a_bolt11(r: Rig) {
    let (status, body) = r.lightning_invoice(2100).await;
    assert_eq!(status, 201, "{body}");
    let instruction = &body["instruction"];
    assert!(instruction["bolt11"].as_str().unwrap().starts_with("lnbcrt"));
    assert!(!instruction["payment_hash"].as_str().unwrap().is_empty());
    assert_eq!((instruction["amount_sats"].as_i64(), instruction["amount_msat"].as_i64()), (Some(2100), Some(2_100_000)));
    let hash = instruction["payment_hash"].as_str().unwrap();
    let (_, status) = r.t.get(&r.t.alice, &format!("/lightning/invoice/{hash}/status")).await;
    assert_eq!(status["status"], "pending");
}

async fn a_settled_topup_credits_once_and_a_replay_does_not_double_credit(r: Rig) {
    let (_, body) = r.lightning_invoice(1000).await;
    let hash = body["instruction"]["payment_hash"].as_str().unwrap().to_string();
    r.settle(&hash).await;
    r.replay_settle(&hash).await;
    r.replay_settle(&hash).await;
    let alice = r.alice().await;
    assert_eq!(r.sats(alice).await, 1_000_000);
    let (_, status) = r.t.get(&r.t.alice, &format!("/lightning/invoice/{hash}/status")).await;
    assert_eq!(status["status"], "paid");
    let row = r.t.state.db.gateway_payment_by_hash(&hash).await.unwrap().unwrap();
    assert_eq!(r.t.state.db.list_transactions_for_reference(&format!("gw-{}", row.public_id)).await.unwrap().len(), 1);
}

async fn an_expired_invoice_credits_nothing(r: Rig) {
    let (_, body) = r.lightning_invoice(1000).await;
    let hash = body["instruction"]["payment_hash"].as_str().unwrap().to_string();
    r.expire(&hash).await;
    r.settle_late_after_expiry(&hash).await;
    let (_, status) = r.t.get(&r.t.alice, &format!("/lightning/invoice/{hash}/status")).await;
    assert_eq!(status["status"], "expired");
    assert_eq!(r.sats(r.alice().await).await, 0);
}

async fn the_account_topup_api_creates_and_polls_a_sats_topup(r: Rig) {
    let (status, created) = r.t.call(&r.t.alice, "POST", "/api/account/topup", r#"{"currency":"sats","amount":1500}"#).await;
    assert_eq!(status, 200, "{created}");
    assert_eq!((created["status"].as_str(), created["amount_milli"].as_i64()), (Some("pending"), Some(1_500_000)));
    let id = created["public_id"].as_str().unwrap().to_string();
    let hash = match &r.backend {
        Backend::Gateway(fake) => format!("hash_{}", fake.last_id()),
        Backend::Lnd(_) => created["payment_hash"].as_str().unwrap().to_string(),
    };
    if let Backend::Gateway(fake) = &r.backend {
        fake.state.lock().unwrap().payments.last_mut().unwrap().method = Some("lightning".into());
    }
    r.settle(&hash).await;
    let (_, polled) = r.t.get(&r.t.alice, &format!("/api/account/topup/{id}")).await;
    assert_eq!(polled["status"], "completed");
    assert_eq!(r.sats(r.alice().await).await, 1_500_000);
}

async fn the_anonymous_flow_derives_the_same_key_as_today(r: Rig) {
    let (status, body) = r.anonymous_invoice(5000).await;
    assert_eq!(status, 200, "{body}");
    let invoice_id = body["invoice_id"].as_str().unwrap().to_string();
    let hash = body["payment_hash"].as_str().unwrap().to_string();
    assert!(body["bolt11"].as_str().unwrap().starts_with("lnbcrt"));
    r.settle(&hash).await;
    let (_, status) = r.t.get("none", &format!("/v2/lightning/invoice/{invoice_id}/status")).await;
    assert_eq!(status["status"], "paid");
    let secret = r.t.state.db.server_secret().await.unwrap();
    assert_eq!(status["api_key"].as_str(), Some(derive_api_key(&secret, &invoice_id).as_str()));
    let (_, again) = r.t.get("none", &format!("/v1/balance/lightning/invoice/{invoice_id}/status")).await;
    assert_eq!(again["api_key"], status["api_key"]);
    let (_, info) = r.t.get(status["api_key"].as_str().unwrap(), "/v1/balance/info").await;
    assert_eq!(info["balance_sats"], 5000);
}

async fn the_lightning_info_advertises_lightning(r: Rig) {
    let (_, info) = r.t.get("none", "/v1/info").await;
    assert_eq!(info["payments"]["methods"], serde_json::json!(["lightning"]));
}

macro_rules! conformance {
    ($($case:ident),* $(,)?) => {
        mod on_the_gateway {
            $(#[tokio::test] async fn $case() { super::$case(super::gateway().await).await })*
        }
        mod on_lnd {
            $(#[tokio::test] async fn $case() { super::$case(super::lnd().await).await })*
        }
    };
}

conformance!(
    a_lightning_topup_is_created_with_a_bolt11,
    a_settled_topup_credits_once_and_a_replay_does_not_double_credit,
    an_expired_invoice_credits_nothing,
    the_account_topup_api_creates_and_polls_a_sats_topup,
    the_anonymous_flow_derives_the_same_key_as_today,
    the_lightning_info_advertises_lightning,
);

#[tokio::test]
async fn on_lnd_the_config_says_lnd_with_sats_only_and_no_cards() {
    let r = lnd().await;
    let (status, config) = r.t.get(&r.t.alice, "/api/account/topup/config").await;
    assert_eq!(status, 200);
    assert_eq!(config["provider"], "lnd");
    assert_eq!(config["cards"], false);
    assert_eq!(config["currencies"], serde_json::json!(["sats"]));
}

#[tokio::test]
async fn on_lnd_card_endpoints_and_card_currencies_say_the_gateway_is_needed() {
    let r = lnd().await;
    let (status, body) = r.t.get(&r.t.alice, "/api/account/payment-methods").await;
    assert_eq!(status, 404);
    assert!(body.to_string().contains("payments.gateway"), "{body}");
    let (status, _) = r.t.call(&r.t.alice, "POST", "/api/account/topup", r#"{"currency":"eur","amount":10}"#).await;
    assert_eq!(status, 400);
    assert_eq!(r.webhook("t=1,v1=00", "{}").await, 404);
}

#[tokio::test]
async fn on_the_gateway_the_config_says_gateway_with_cards() {
    let r = gateway().await;
    let (_, config) = r.t.get(&r.t.alice, "/api/account/topup/config").await;
    assert_eq!((config["provider"].as_str(), config["cards"].as_bool()), (Some("gateway"), Some(true)));
}

#[tokio::test]
async fn on_lnd_a_refund_pays_the_users_invoice_from_the_node() {
    let r = lnd().await;
    let alice = r.alice().await;
    r.t.state.payments_state.as_ref().unwrap().balance_service.credit(alice, 5_000_000, "deposit", "seed").await.unwrap();
    let invoice = crate::payments::routstr::test_invoice(2_000_000);
    let (status, body) = r.t.call(&r.t.alice, "POST", "/v1/balance/refund", &format!(r#"{{"invoice":"{invoice}"}}"#)).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(r.sats(alice).await, 3_000_000);
    let Backend::Lnd(node) = &r.backend else { unreachable!() };
    assert_eq!(node.state.lock().unwrap().paid, vec![invoice]);
}
