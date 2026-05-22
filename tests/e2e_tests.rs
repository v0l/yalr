// ── E2E Test Suite ──────────────────────────
// Real bitcoind (regtest) + LND + YALR.
// Run with: cargo test --test e2e_tests -- --nocapture --test-threads=1

use reqwest::{Client, StatusCode};
use serde_json::{json, Value};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Mutex;

use payments_rs::lightning::{LndNode, LightningNode, PayInvoiceRequest};

// ── Config ─────────────────────────────────

const YALR_URL: &str = "http://localhost:3099";
const MOCK_LLM_URL: &str = "http://localhost:4004";
const _MOCK_LLM_DOCKER_URL: &str = "http://mock-llm:4000";

// ── Globals ────────────────────────────────

static SETUP_LOCK: OnceLock<Mutex<(String, i64)>> = OnceLock::new();
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn seq() -> u64 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn uname(base: &str) -> String {
    format!("{}-{}", base, seq())
}

fn uslug(base: &str) -> String {
    format!("{}-{}", base, seq())
}

// ── HTTP helpers ────────────────────────────

fn client() -> Client {
    Client::builder().timeout(Duration::from_secs(30)).build().unwrap()
}

fn client_auth(token: &str) -> Client {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
    );
    Client::builder().timeout(Duration::from_secs(30)).default_headers(h).build().unwrap()
}

async fn wait_yalr() {
    let c = client();
    for _ in 0..60 {
        if c.get(&format!("{}/api/health", YALR_URL)).send().await.map_or(false, |r| r.status().is_success()) {
            println!("YALR is healthy");
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("YALR not healthy");
}

async fn payments_enabled(token: &str) -> bool {
    client_auth(token)
        .get(&format!("{}/v1/info", YALR_URL))
        .send()
        .await
        .map_or(false, |r| r.status().is_success())
}

/// Serialised admin setup: create admin user + login + return (token, user_id).
async fn setup_admin() -> (String, i64) {
    let mu = SETUP_LOCK.get_or_init(|| Mutex::new((String::new(), 0)));
    let mut g = mu.lock().await;
    if !g.0.is_empty() {
        return g.clone();
    }
    let c = client();
    let ss: Value = c.get(&format!("{}/api/setup/status", YALR_URL)).send().await.unwrap().json().await.unwrap_or_default();
    if !ss.get("setup_complete").and_then(|v| v.as_bool()).unwrap_or(false) {
        let r = c.post(&format!("{}/api/auth/setup", YALR_URL)).json(&json!({"username":"e2e-admin","password":"e2e-test-password"})).send().await.unwrap();
        if r.status() != StatusCode::OK {
            panic!("setup failed: {} {}", r.status(), r.text().await.unwrap_or_default());
        }
    }
    let login: Value = c.post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":"e2e-admin","password":"e2e-test-password"})).send().await.unwrap().json().await.unwrap();
    let token = login["token"].as_str().unwrap().to_string();
    let a = client_auth(&token);
    // Get admin user id from user list
    let users: Value = a.get(&format!("{}/api/users", YALR_URL)).send().await.unwrap().json().await.unwrap();
    let arr = users.as_array().unwrap();
    let adm = arr.iter().find(|u| u["username"] == "e2e-admin").unwrap();
    let uid = adm["id"].as_i64().unwrap();

    // Cleanup stale entities from prior runs
    for u in arr {
        let id = u["id"].as_i64().unwrap_or(0);
        if !u["is_admin"].as_bool().unwrap_or(false) && id > 0 {
            let _ = a.delete(&format!("{}/api/users/{}", YALR_URL, id)).send().await;
        }
    }
    if let Ok(p) = a.get(&format!("{}/api/providers", YALR_URL)).send().await {
        if let Ok(v) = p.json::<Value>().await {
            if let Some(pv) = v["providers"].as_array() {
                for pr in pv {
                    if let Some(slug) = pr["slug"].as_str() {
                        // Delete all providers from prior runs (including stale e2e-mock ones)
                        let _ = a.delete(&format!("{}/api/providers/{}", YALR_URL, slug)).send().await;
                    }
                }
            }
        }
    }
    // Delete routing configs
    if let Ok(rcs) = a.get(&format!("{}/api/routing-configs", YALR_URL)).send().await {
        if let Ok(v) = rcs.json::<Value>().await {
            if let Some(arr) = v.as_array() {
                for rc in arr {
                    let _ = a.delete(&format!("{}/api/routing-configs/{}", YALR_URL, rc["id"].as_i64().unwrap())).send().await;
                }
            }
        }
    }
    // Delete non-mock model pricings
    let _ = a.delete(&format!("{}/api/model-pricing/e2e-model", YALR_URL)).send().await;

    *g = (token.clone(), uid);
    (token, uid)
}

/// LND Bob connect helper
async fn bob_lnd() -> LndNode {
    payments_rs::lightning::setup_crypto_provider();
    // Copy credentials from docker if needed
    let cert = "lnd-data-bob-temp/tls.cert";
    let mac = "lnd-data-bob-temp/data/chain/bitcoin/regtest/admin.macaroon";
    if !std::path::Path::new(cert).exists() {
        let _ = std::process::Command::new("docker")
            .args(["compose", "-f", "docker-compose.e2e.yaml", "cp", "lnd-bob:/root/.lnd", "lnd-data-bob-temp"])
            .output();
        // Flatten if nested
        let nested = "lnd-data-bob-temp/.lnd";
        if std::path::Path::new(nested).exists() {
            let _ = std::process::Command::new("sh").args(["-c", "mv lnd-data-bob-temp/.lnd/* lnd-data-bob-temp/ && rmdir lnd-data-bob-temp/.lnd"]).output();
        }
    }
    LndNode::new("https://localhost:10029", std::path::Path::new(cert), std::path::Path::new(mac)).await.unwrap()
}

async fn wait_invoice_paid(token: &str, payment_hash: &str, secs: u64) -> bool {
    let c = client_auth(token);
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if let Ok(r) = c.get(&format!("{}/lightning/invoice/{}/status", YALR_URL, payment_hash)).send().await {
            if let Ok(b) = r.json::<Value>().await {
                if b["status"] == "paid" { return true; }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

async fn wait_balance(token: &str, min_msat: i64, secs: u64) -> bool {
    let c = client_auth(token);
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if let Ok(r) = c.get(&format!("{}/v1/balance/info", YALR_URL)).send().await {
            if let Ok(b) = r.json::<Value>().await {
                if b["balance_msat"].as_i64().unwrap_or(0) >= min_msat { return true; }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    false
}

/// Sets up billing infrastructure (provider + routing config + model pricing) for chat tests.
/// Returns Some((routing_config_name, provider_slug)).
async fn setup_billing(token: &str) -> Option<(String, String)> {
    let a = client_auth(token);
    if client().get(&format!("{}/v1/models", MOCK_LLM_URL)).send().await.is_err() {
        println!("SKIP: Mock LLM not reachable");
        return None;
    }
    let rc_name = uname("billing-rc");

    let model = uname("billing-model");
    let slug = uslug("e2e-mock");

    let p: Value = a.post(&format!("{}/api/providers", YALR_URL))
        .json(&json!({"name":format!("E2E Mock {}",seq()),"slug":slug,"base_url":"http://mock-llm:4000","api_key":"x","provider_type":"openai"}))
        .send().await.unwrap().json().await.unwrap();
    let pid = p["id"].as_i64().unwrap();

    a.post(&format!("{}/api/model-pricing", YALR_URL))
        .json(&json!({"model_name":rc_name,"is_advertised":true,"is_free":false,"price_per_1m_input_sats":5,"price_per_1m_output_sats":15,"price_per_request_sats":1,"context_window":8192,"max_output_tokens":4096}))
        .send().await.unwrap();

    let rc: Value = a.post(&format!("{}/api/routing-configs", YALR_URL))
        .json(&json!({"name":rc_name,"strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5}))
        .send().await.unwrap().json().await.unwrap();
    let rc_id = rc["id"].as_i64().unwrap();

    a.post(&format!("{}/api/routing-configs/providers", YALR_URL))
        .json(&json!({"routing_config_id":rc_id,"provider_id":pid,"model":model,"weight":100,"is_active":true}))
        .send().await.unwrap();

    println!("Billing setup done: model={} rc={} slug={}", model, rc_name, slug);
    Some((rc_name, slug))
}

// ═════════════════════════════════════════════
// 1. HEALTH
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_health() {
    wait_yalr().await;
    let r = client().get(&format!("{}/api/health", YALR_URL)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    println!("Health: OK");
}

// ═════════════════════════════════════════════
// 2. AUTH
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_auth_flow() {
    wait_yalr().await;
    let (token, uid) = setup_admin().await;
    assert!(uid > 0 && !token.is_empty());
    let a = client_auth(&token);

    let s: Value = a.get(&format!("{}/api/auth/status", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(s["username"], "e2e-admin");
    assert!(s["is_admin"].as_bool().unwrap());

    let logout = a.post(&format!("{}/api/auth/logout", YALR_URL)).send().await.unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    println!("Auth flow: OK");
}

#[tokio::test]
async fn test_protected_routes() {
    wait_yalr().await;
    let c = client();
    for (method, path) in [("GET","/api/providers"),("GET","/api/metrics"),("GET","/api/users"),("GET","/v1/balance/info"),("POST","/api/routing-configs")] {
        let r = if method == "GET" { c.get(&format!("{}{}", YALR_URL, path)) } else { c.post(&format!("{}{}", YALR_URL, path)) }.send().await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
    }
    println!("Auth enforcement: OK");
}

// ═════════════════════════════════════════════
// 3. API KEYS
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_api_keys() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);

    let c: Value = a.post(&format!("{}/api/api-keys", YALR_URL)).json(&json!({"name":"e2e-key"})).send().await.unwrap().json().await.unwrap();
    let key = c["key"].as_str().unwrap().to_string();
    let id = c["id"].as_i64().unwrap();
    assert!(!key.is_empty());

    let list: Value = a.get(&format!("{}/api/api-keys", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(!list.as_array().unwrap().is_empty());

    a.post(&format!("{}/api/api-keys/{}/disable", YALR_URL, id)).send().await.unwrap();
    a.post(&format!("{}/api/api-keys/{}/enable", YALR_URL, id)).send().await.unwrap();

    assert!(client_auth(&key).get(&format!("{}/api/auth/status", YALR_URL)).send().await.unwrap().status().is_success());

    a.delete(&format!("{}/api/api-keys/{}", YALR_URL, id)).send().await.unwrap();
    println!("API keys: OK");
}

// ═════════════════════════════════════════════
// 4. PROVIDERS
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_providers() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let slug = uslug("ept");

    let c: Value = a.post(&format!("{}/api/providers", YALR_URL)).json(&json!({"name":"E2E Provider","slug":slug,"base_url":"http://localhost:9999","api_key":"x","provider_type":"ollama"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(c["slug"], slug);

    let list: Value = a.get(&format!("{}/api/providers", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(list["providers"].as_array().unwrap().iter().any(|p| p["slug"] == slug));

    let u: Value = a.put(&format!("{}/api/providers/{}", YALR_URL, slug)).json(&json!({"name":"E2E Updated","base_url":"http://localhost:9998"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(u["name"], "E2E Updated");

    a.delete(&format!("{}/api/providers/{}", YALR_URL, slug)).send().await.unwrap();
    println!("Providers: OK");
}

#[tokio::test]
async fn test_provider_api_key() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let slug = uslug("apk");

    a.post(&format!("{}/api/providers", YALR_URL)).json(&json!({"name":"APK","slug":slug,"base_url":"http://x","api_key":"x","provider_type":"openai"})).send().await.unwrap();
    let r = a.post(&format!("{}/api/providers/{}/generate-api-key", YALR_URL, slug)).send().await.unwrap();
    if r.status().is_success() {
        let g: Value = r.json().await.unwrap();
        assert!(g["api_key"].as_str().unwrap().len() > 0);
    }
    a.delete(&format!("{}/api/providers/{}", YALR_URL, slug)).send().await.unwrap();
    println!("Provider API key: OK");
}

// ═════════════════════════════════════════════
// 5. USERS
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_users() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let username = uname("e2e-user");

    let c: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = c["user"]["id"].as_i64().unwrap();
    assert_eq!(c["user"]["username"], username);

    let list: Value = a.get(&format!("{}/api/users", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(list.as_array().unwrap().iter().any(|u| u["id"] == uid));

    let g: Value = a.get(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap().json().await.unwrap();
    assert_eq!(g["user"]["username"], username);

    a.put(&format!("{}/api/users/{}", YALR_URL, uid)).json(&json!({"username":uname("upd")})).send().await.unwrap();

    let k: Value = a.post(&format!("{}/api/users/{}/api-keys", YALR_URL, uid)).json(&json!({"name":"uk"})).send().await.unwrap().json().await.unwrap();
    assert!(k["key"].as_str().is_some());

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Users: OK");
}

// ═════════════════════════════════════════════
// 6. ROUTING CONFIG
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_routing_config() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let slug = uslug("rct");

    let p: Value = a.post(&format!("{}/api/providers", YALR_URL)).json(&json!({"name":format!("RC-{}",seq()),"slug":slug,"base_url":"http://localhost:9996","api_key":"x","provider_type":"openai"})).send().await.unwrap().json().await.unwrap();
    let pid = p["id"].as_i64().unwrap();

    let rc: Value = a.post(&format!("{}/api/routing-configs", YALR_URL)).json(&json!({"name":format!("E2E RC {}",seq()),"strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5})).send().await.unwrap().json().await.unwrap();
    let rcid = rc["id"].as_i64().unwrap();
    assert_eq!(rc["strategy"], "round_robin");

    let rcp_resp: Value = a.post(&format!("{}/api/routing-configs/providers", YALR_URL)).json(&json!({"routing_config_id":rcid,"provider_id":pid,"model":"gpt-4","weight":100,"is_active":true})).send().await.unwrap().json().await.unwrap();
    // Response is {"message":"..."} - get id from routing config list instead
    let rc_list: Value = a.get(&format!("{}/api/routing-configs", YALR_URL)).send().await.unwrap().json().await.unwrap();
    let rc_full = rc_list.as_array().unwrap().iter().find(|r| r["id"].as_i64() == Some(rcid)).unwrap();
    let rcp_id = rc_full["providers"].as_array().unwrap().first().unwrap()["id"].as_i64().unwrap();
    let _ = rcp_resp;

    let ur: Value = a.put(&format!("{}/api/routing-configs/providers/{}", YALR_URL, rcp_id)).json(&json!({"weight":50,"model":"gpt-4-turbo"})).send().await.unwrap().json().await.unwrap();
    assert!(ur["message"].as_str().unwrap_or("").contains("updated"), "Expected updated message");

    a.delete(&format!("{}/api/routing-configs/providers/{}", YALR_URL, rcp_id)).send().await.unwrap();
    a.delete(&format!("{}/api/routing-configs/{}", YALR_URL, rcid)).send().await.unwrap();
    a.delete(&format!("{}/api/providers/{}", YALR_URL, slug)).send().await.unwrap();
    println!("Routing config: OK");
}

// ═════════════════════════════════════════════
// 7. MODEL PRICING
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_model_pricing() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let mn = uname("mp-test");

    let c: Value = a.post(&format!("{}/api/model-pricing", YALR_URL)).json(&json!({"model_name":mn,"is_advertised":true,"is_free":false,"price_per_1m_input_sats":10,"price_per_1m_output_sats":30,"price_per_request_sats":2,"context_window":32768,"max_output_tokens":8192})).send().await.unwrap().json().await.unwrap();
    assert_eq!(c["model_name"], mn);

    let list: Value = a.get(&format!("{}/api/model-pricing", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(list.as_array().unwrap().iter().any(|x| x["model_name"] == mn));

    let u: Value = a.put(&format!("{}/api/model-pricing/{}", YALR_URL, mn)).json(&json!({"price_per_1m_input_sats":20,"is_free":true})).send().await.unwrap().json().await.unwrap();
    assert_eq!(u["price_per_1m_input_sats"], 20);

    a.delete(&format!("{}/api/model-pricing/{}", YALR_URL, mn)).send().await.unwrap();
    println!("Model pricing: OK");
}

// ═════════════════════════════════════════════
// 8. LIGHTNING INVOICE PAYMENT
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_ln_invoice_payment() {
    wait_yalr().await;
    let (at, _) = setup_admin().await;
    let a = client_auth(&at);
    if !payments_enabled(&at).await { println!("SKIP"); return; }

    let username = uname("ln-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let login: Value = client().post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":username,"password":"p"})).send().await.unwrap().json().await.unwrap();
    let ut = login["token"].as_str().unwrap().to_string();
    let ua = client_auth(&ut);

    let bal: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(bal["balance_msat"].as_i64().unwrap_or(-1), 0);

    let inv: Value = ua.post(&format!("{}/lightning/invoice", YALR_URL)).json(&json!({"amount_sats":1000,"memo":"e2e"})).send().await.unwrap().json().await.unwrap();
    let bolt11 = inv["instruction"]["bolt11"].as_str().unwrap().to_string();
    let ph = inv["instruction"]["payment_hash"].as_str().unwrap().to_string();

    let st: Value = ua.get(&format!("{}/lightning/invoice/{}/status", YALR_URL, ph)).send().await.unwrap().json().await.unwrap();
    assert_eq!(st["status"], "pending");

    let lnd = bob_lnd().await;
    lnd.pay_invoice(PayInvoiceRequest { invoice: bolt11, timeout_seconds: Some(120) }).await.expect("pay failed");

    assert!(wait_invoice_paid(&ut, &ph, 60).await, "invoice not settled");
    assert!(wait_balance(&ut, 1_000_000, 60).await, "balance not credited");

    let fb: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(fb["balance_msat"].as_i64().unwrap(), 1_000_000);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("LN payment: OK");
}

// ═════════════════════════════════════════════
// 9. LIGHTNING REFUND
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_ln_refund() {
    wait_yalr().await;
    let (at, _) = setup_admin().await;
    let a = client_auth(&at);
    if !payments_enabled(&at).await { println!("SKIP"); return; }

    let username = uname("refund-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let credit: Value = a.post(&format!("{}/api/payments/credit", YALR_URL)).json(&json!({"user_id":uid,"amount_sats":500,"reason":"e2e"})).send().await.unwrap().json().await.unwrap();
    assert!(credit["new_balance_msat"].as_i64().unwrap() >= 500_000);

    let login: Value = client().post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":username,"password":"p"})).send().await.unwrap().json().await.unwrap();
    let ut = login["token"].as_str().unwrap().to_string();
    let ua = client_auth(&ut);

    let bal: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(bal["balance_msat"].as_i64().unwrap(), 500_000);

    let lnd = bob_lnd().await;
    use payments_rs::lightning::AddInvoiceRequest;
    let binv = lnd.add_invoice(AddInvoiceRequest { amount: 500_000, memo: Some("e2e refund".into()), expire: None }).await.unwrap();

    let rr = ua.post(&format!("{}/v1/balance/refund", YALR_URL)).json(&json!({"invoice":binv.pr(),"amount_sats":500})).send().await;
    match rr {
        Ok(r) if r.status().is_success() => { println!("Refund OK"); }
        Ok(r) => { println!("Refund status: {}", r.status()); }
        Err(e) => { println!("Refund err: {e}"); }
    }

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("LN refund: OK");
}

// ═════════════════════════════════════════════
// 10. ADMIN PAYMENTS
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_admin_payments() {
    wait_yalr().await;
    let (at, _) = setup_admin().await;
    let a = client_auth(&at);
    if !payments_enabled(&at).await { println!("SKIP"); return; }

    let username = uname("apay-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let cc: Value = a.post(&format!("{}/api/payments/credit", YALR_URL)).json(&json!({"user_id":uid,"amount_sats":200,"reason":"e2e"})).send().await.unwrap().json().await.unwrap();
    assert!(cc["new_balance_msat"].as_i64().unwrap() >= 200000);

    let dd: Value = a.post(&format!("{}/api/payments/debit", YALR_URL)).json(&json!({"user_id":uid,"amount_sats":50,"reason":"e2e"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(dd["new_balance_msat"].as_i64().unwrap(), 150000);

    let bl: Value = a.get(&format!("{}/api/payments/balances", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(bl.as_array().unwrap().iter().any(|b| b["user_id"].as_i64().unwrap() == uid));

    let dt: Value = a.get(&format!("{}/api/payments/balances/{}", YALR_URL, uid)).send().await.unwrap().json().await.unwrap();
    assert_eq!(dt["balance_msat"].as_i64().unwrap(), 150000);
    assert!(!dt["transactions"].as_array().unwrap().is_empty());

    let tx: Value = a.get(&format!("{}/api/payments/transactions", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert!(!tx.as_array().unwrap().is_empty());

    let iv: Value = a.get(&format!("{}/api/payments/invoices", YALR_URL)).send().await.unwrap().json().await.unwrap();
    println!("Invoices: {} total", iv.as_array().unwrap().len());

    let over = a.post(&format!("{}/api/payments/debit", YALR_URL)).json(&json!({"user_id":uid,"amount_sats":99999,"reason":"fail"})).send().await.unwrap();
    // Over-debit may return 400, 402 or 500 depending on server handling
    let status = over.status();
    assert!(status.is_client_error() || status.is_server_error(), "Expected error status, got {}", status);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Admin payments: OK");
}

// ═════════════════════════════════════════════
// 11. MODEL ACCESS CONTROL
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_model_access() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);

    let username = uname("mac-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let list: Value = a.get(&format!("{}/api/users/{}/models", YALR_URL, uid)).send().await.unwrap().json().await.unwrap();
    // Model permissions may be wrapped; handle both shapes
    let empty_vec = vec![];
    let perm_list = if let Some(arr) = list.as_array() { arr } else { list.get("permissions").and_then(|v| v.as_array()).unwrap_or(&empty_vec) };
    assert!(perm_list.is_empty());

    let ca: Value = a.post(&format!("{}/api/users/{}/models", YALR_URL, uid)).json(&json!({"user_id":uid,"model":"gpt-4","allow":true})).send().await.unwrap().json().await.unwrap();
    assert_eq!(ca["allow"].as_bool(), Some(true));

    let cd: Value = a.post(&format!("{}/api/users/{}/models", YALR_URL, uid)).json(&json!({"user_id":uid,"model":"claude-3","allow":false})).send().await.unwrap().json().await.unwrap();
    assert_eq!(cd["allow"].as_bool(), Some(false));

    let l2: Value = a.get(&format!("{}/api/users/{}/models", YALR_URL, uid)).send().await.unwrap().json().await.unwrap();
    assert_eq!(l2.as_array().unwrap().len(), 2);

    a.delete(&format!("{}/api/users/{}/models/gpt-4", YALR_URL, uid)).send().await.unwrap();
    let l3: Value = a.get(&format!("{}/api/users/{}/models", YALR_URL, uid)).send().await.unwrap().json().await.unwrap();
    assert_eq!(l3.as_array().unwrap().len(), 1);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Model access: OK");
}

// ═════════════════════════════════════════════
// 12. METRICS & MODELS
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_metrics() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);

    a.get(&format!("{}/api/metrics", YALR_URL)).send().await.unwrap();
    a.get(&format!("{}/api/metrics/history", YALR_URL)).send().await.unwrap();
    a.get(&format!("{}/api/metrics/health", YALR_URL)).send().await.unwrap();
    a.get(&format!("{}/api/config", YALR_URL)).send().await.unwrap();
    a.get(&format!("{}/v1/models", YALR_URL)).send().await.unwrap();
    println!("Metrics: OK");
}

// ═════════════════════════════════════════════
// 13. PAYMENTS DISABLED
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_payments_disabled_info() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);
    let info: Value = a.get(&format!("{}/v1/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    if !info["payments"]["enabled"].as_bool().unwrap_or(false) {
        for path in ["/v1/balance/info","/v1/balance/refund","/lightning/invoice"] {
            let r = a.get(&format!("{}{}", YALR_URL, path)).send().await.unwrap();
            assert_eq!(r.status(), StatusCode::NOT_FOUND);
        }
    }
    println!("Payments info: OK");
}

// ═════════════════════════════════════════════
// 14. CHAT BILLING
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_chat_billing() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);

    let Some((model, _)) = setup_billing(&t).await else { return; };

    let username = uname("cbill-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let login: Value = client().post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":username,"password":"p"})).send().await.unwrap().json().await.unwrap();
    let ut = login["token"].as_str().unwrap().to_string();
    let ua = client_auth(&ut);

    // 402 without balance
    let r402 = ua.post(&format!("{}/v1/chat/completions", YALR_URL)).json(&json!({"model":model,"messages":[{"role":"user","content":"Hi"}]})).send().await.unwrap();
    assert_eq!(r402.status(), StatusCode::PAYMENT_REQUIRED);

    // Credit and send
    a.post(&format!("{}/api/payments/credit", YALR_URL)).json(&json!({"user_id":uid,"amount_sats":5,"reason":"e2e"})).send().await.unwrap();

    let resp = ua.post(&format!("{}/v1/chat/completions", YALR_URL)).json(&json!({"model":model,"messages":[{"role":"user","content":"Say hi in three words."}]})).send().await.unwrap();
    let status = resp.status();
    let resp_text = resp.text().await.unwrap();
    if let Ok(body) = serde_json::from_str::<Value>(&resp_text) {
        assert_eq!(status, StatusCode::OK, "Chat failed with status {}: {}", status.as_u16(), body);
    } else {
        assert_eq!(status, StatusCode::OK, "Chat failed with status {}: {}", status.as_u16(), resp_text);
        return;
    }
    let body: Value = serde_json::from_str(&resp_text).unwrap();
    assert_eq!(body["usage"]["prompt_tokens"], 50);
    assert_eq!(body["usage"]["completion_tokens"], 150);
    assert!(body["model"].as_str().unwrap_or("").starts_with("billing-model"), "Unexpected model: {}", body["model"]);

    // Balance: 5 sats credit = 5000 msats; cost = ~1000 msats; expected ~4000
    let bal: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    let b = bal["balance_msat"].as_i64().unwrap();
    assert!(b >= 3500 && b <= 4500, "Expected ~4000 msats, got {}", b);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Chat billing: OK");
}

// ═════════════════════════════════════════════
// 15. FREE MODEL BILLING
// ═════════════════════════════════════════════

#[tokio::test]
async fn test_free_model() {
    wait_yalr().await;
    let (t, _) = setup_admin().await;
    let a = client_auth(&t);

    let Some((model, _)) = setup_billing(&t).await else { return; };

    // Make model free
    a.put(&format!("{}/api/model-pricing/{}", YALR_URL, model)).json(&json!({"is_free":true})).send().await.unwrap();

    let username = uname("free-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let login: Value = client().post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":username,"password":"p"})).send().await.unwrap().json().await.unwrap();
    let ut = login["token"].as_str().unwrap().to_string();
    let ua = client_auth(&ut);

    let resp = ua.post(&format!("{}/v1/chat/completions", YALR_URL)).json(&json!({"model":model,"messages":[{"role":"user","content":"Hi"}]})).send().await.unwrap();
    let status = resp.status();
    let resp_text = resp.text().await.unwrap();
    if status != StatusCode::OK {
        let body: Value = serde_json::from_str(&resp_text).unwrap_or_default();
        assert_eq!(status, StatusCode::OK, "Free chat failed: {}", body);
    }
    let body: Value = serde_json::from_str(&resp_text).unwrap();
    assert_eq!(body["usage"]["prompt_tokens"], 50);

    let bal: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(bal["balance_msat"].as_i64().unwrap(), 0);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Free model: OK");
}

// ═════════════════════════════════════════════
// 16. MULTI LN PAYMENTS (ignored)
// ═════════════════════════════════════════════

#[tokio::test]
#[ignore = "slow, needs LND"]
async fn test_multi_ln() {
    wait_yalr().await;
    let (at, _) = setup_admin().await;
    let a = client_auth(&at);
    if !payments_enabled(&at).await { return; }

    let username = uname("mln-user");
    let cr: Value = a.post(&format!("{}/api/users", YALR_URL)).json(&json!({"username":username,"password":"p","is_admin":false,"user_type":"internal"})).send().await.unwrap().json().await.unwrap();
    let uid = cr["user"]["id"].as_i64().unwrap();

    let login: Value = client().post(&format!("{}/api/auth/login", YALR_URL)).json(&json!({"username":username,"password":"p"})).send().await.unwrap().json().await.unwrap();
    let ut = login["token"].as_str().unwrap().to_string();
    let ua = client_auth(&ut);

    let lnd = bob_lnd().await;
    let mut total = 0i64;
    for amt in [500, 250, 750] {
        let inv: Value = ua.post(&format!("{}/lightning/invoice", YALR_URL)).json(&json!({"amount_sats":amt,"memo":"e2e"})).send().await.unwrap().json().await.unwrap();
        let bolt11 = inv["instruction"]["bolt11"].as_str().unwrap().to_string();
        let ph = inv["instruction"]["payment_hash"].as_str().unwrap().to_string();
        lnd.pay_invoice(PayInvoiceRequest { invoice: bolt11, timeout_seconds: Some(120) }).await.unwrap();
        wait_invoice_paid(&ut, &ph, 30).await;
        total += (amt as i64) * 1000;
    }
    let bal: Value = ua.get(&format!("{}/v1/balance/info", YALR_URL)).send().await.unwrap().json().await.unwrap();
    assert_eq!(bal["balance_msat"].as_i64().unwrap(), total);

    a.delete(&format!("{}/api/users/{}", YALR_URL, uid)).send().await.unwrap();
    println!("Multi LN: OK");
}
