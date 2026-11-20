use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const WEBHOOK_SECRET: &str = "whsec_test";

#[derive(Clone, Debug)]
pub struct FakePayment {
    pub id: String,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub reference: String,
    pub customer_ref: String,
    pub method: Option<String>,
    pub save_method: bool,
}

#[derive(Default)]
pub struct FakeState {
    pub payments: Vec<FakePayment>,
    pub idempotency: HashMap<String, String>,
    pub saved: Vec<(i64, String)>,
    pub refunds: Vec<(String, i64, String)>,
    pub refuse_refunds: bool,
    pub decline_charges: bool,
    pub requests: Vec<String>,
    pub btc_usd: Option<f64>,
    pub rate_calls: usize,
}

#[derive(Clone)]
pub struct FakeGateway {
    pub url: String,
    pub state: Arc<Mutex<FakeState>>,
}

fn view(base: &str, p: &FakePayment) -> Value {
    let attempts = match p.method.as_deref() {
        Some("lightning") => json!([{
            "id": 1, "provider_id": 1, "method": "lightning", "status": "pending",
            "details": {"bolt11": format!("lnbcrt{}", p.id), "payment_hash": format!("hash_{}", p.id)}
        }]),
        Some(other) => json!([{
            "id": 1, "provider_id": 1, "method": other, "status": "pending",
            "details": {"checkout_url": format!("https://checkout.example/{}", p.id)}
        }]),
        None => json!([]),
    };
    json!({
        "id": p.id, "reference": p.reference, "amount": p.amount, "currency": p.currency,
        "status": p.status, "pay_url": format!("{base}/pay/{}", p.id),
        "expires": "2099-01-01T00:00:00Z", "attempts": attempts,
    })
}

type Shared = (Arc<Mutex<FakeState>>, String);

async fn create(State((state, base)): State<Shared>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut s = state.lock().unwrap();
    s.requests.push("POST /v1/payments".into());
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    if let Some(id) = s.idempotency.get(&key).cloned() {
        let p = s.payments.iter().find(|p| p.id == id).unwrap().clone();
        return (StatusCode::OK, Json(view(&base, &p)));
    }
    let id = format!("gw{}", s.payments.len() + 1);
    let payment = FakePayment {
        id: id.clone(),
        amount: body["amount"].as_i64().unwrap_or_default(),
        currency: body["currency"].as_str().unwrap_or_default().to_string(),
        status: "pending".into(),
        reference: body["reference"].as_str().unwrap_or_default().to_string(),
        customer_ref: body["customer_ref"].as_str().unwrap_or_default().to_string(),
        method: None,
        save_method: false,
    };
    s.idempotency.insert(key, id);
    let out = view(&base, &payment);
    s.payments.push(payment);
    (StatusCode::CREATED, Json(out))
}

async fn attempt(State((state, base)): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut s = state.lock().unwrap();
    s.requests.push(format!("POST /v1/payments/{id}/attempts"));
    let Some(p) = s.payments.iter_mut().find(|p| p.id == id) else { return (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))) };
    p.method = body["method"].as_str().map(str::to_string);
    p.save_method = body["save_method"].as_bool().unwrap_or(false);
    (StatusCode::OK, Json(view(&base, p)))
}

async fn get_payment(State((state, base)): State<Shared>, Path(id): Path<String>) -> (StatusCode, Json<Value>) {
    let s = state.lock().unwrap();
    match s.payments.iter().find(|p| p.id == id) {
        Some(p) => (StatusCode::OK, Json(view(&base, p))),
        None => (StatusCode::NOT_FOUND, Json(json!({"error": "not found"}))),
    }
}

async fn rates(State((state, _)): State<Shared>) -> (StatusCode, Json<Value>) {
    let mut s = state.lock().unwrap();
    s.rate_calls += 1;
    match s.btc_usd {
        Some(usd) => (StatusCode::OK, Json(json!({"btc": {"USD": usd, "EUR": usd * 0.9}}))),
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "no rates"}))),
    }
}

async fn saved(State((state, _)): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    let s = state.lock().unwrap();
    let customer = q.get("customer_ref").cloned().unwrap_or_default();
    Json(Value::Array(
        s.saved
            .iter()
            .filter(|(_, c)| *c == customer)
            .map(|(id, c)| json!({"id": id, "customer_ref": c, "provider_id": 1, "brand": "visa", "last4": "4242", "exp_month": 12, "exp_year": 2030}))
            .collect(),
    ))
}

async fn delete_saved(State((state, _)): State<Shared>, Path(id): Path<i64>) -> Json<Value> {
    state.lock().unwrap().saved.retain(|(i, _)| *i != id);
    Json(json!({"deleted": true}))
}

async fn charge(State((state, base)): State<Shared>, Path(_): Path<i64>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut s = state.lock().unwrap();
    s.requests.push("POST /v1/saved-methods/charge".into());
    let key = headers.get("idempotency-key").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let id = format!("gw{}", s.payments.len() + 1);
    let status = if s.decline_charges { "failed" } else { "paid" };
    let payment = FakePayment {
        id: id.clone(),
        amount: body["amount"].as_i64().unwrap_or_default(),
        currency: body["currency"].as_str().unwrap_or_default().to_string(),
        status: status.into(),
        reference: body["reference"].as_str().unwrap_or_default().to_string(),
        customer_ref: body["customer_ref"].as_str().unwrap_or_default().to_string(),
        method: Some("revolut".into()),
        save_method: false,
    };
    s.idempotency.insert(key, id);
    let out = view(&base, &payment);
    s.payments.push(payment);
    (StatusCode::CREATED, Json(out))
}

async fn refund(State((state, _)): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut s = state.lock().unwrap();
    if s.refuse_refunds {
        return (StatusCode::CONFLICT, Json(json!({"error": "refund refused"})));
    }
    s.refunds.push((id, body["amount"].as_i64().unwrap_or_default(), body["destination"].as_str().unwrap_or_default().to_string()));
    (StatusCode::OK, Json(json!({"status": "pending"})))
}

impl FakeGateway {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/v1/payments", post(create))
            .route("/v1/payments/:id", get(get_payment))
            .route("/v1/payments/:id/attempts", post(attempt))
            .route("/v1/payments/:id/refund", post(refund))
            .route("/v1/rates", get(rates))
            .route("/v1/saved-methods", get(saved))
            .route("/v1/saved-methods/:id", delete(delete_saved))
            .route("/v1/saved-methods/:id/charge", post(charge))
            .with_state((state.clone(), url.clone()));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state }
    }

    pub fn payment(&self, id: &str) -> FakePayment {
        self.state.lock().unwrap().payments.iter().find(|p| p.id == id).cloned().unwrap()
    }

    pub fn last_id(&self) -> String {
        self.state.lock().unwrap().payments.last().unwrap().id.clone()
    }

    pub fn set_status(&self, id: &str, status: &str) {
        self.state.lock().unwrap().payments.iter_mut().find(|p| p.id == id).unwrap().status = status.to_string();
    }

    pub fn add_saved(&self, id: i64, customer: &str) {
        self.state.lock().unwrap().saved.push((id, customer.to_string()));
    }

    pub fn event(&self, kind: &str, id: &str) -> (String, String) {
        self.event_signed_with(WEBHOOK_SECRET, chrono::Utc::now().timestamp(), kind, id)
    }

    pub fn event_signed_with(&self, secret: &str, timestamp: i64, kind: &str, id: &str) -> (String, String) {
        let p = self.payment(id);
        let body = json!({"id": format!("evt_{kind}_{id}"), "type": kind, "created": "2026-11-19T10:00:00Z",
            "data": {"payment": view(&self.url, &p), "detail": {}}})
        .to_string();
        let header = format!("t={timestamp},v1={}", payment_gateway_client::sign_webhook(body.as_bytes(), secret, timestamp));
        (header, body)
    }
}
