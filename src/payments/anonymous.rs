use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::auth::api_key::{get_last_four, hash_api_key};
use crate::db::{random_hex, Database, LightningInvoiceRow, NewApiKey, NewUser, UserType};
use crate::payments::instructions::PaymentInstruction;
use crate::state::AppState;

const MIN_SATS: u64 = 100;
const MAX_SATS: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    Create,
    Topup,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Purpose::Create => "create",
            Purpose::Topup => "topup",
        }
    }
}

#[derive(Deserialize)]
pub struct InvoiceRequest {
    pub amount_sats: u64,
    pub purpose: Purpose,
}

#[derive(Deserialize)]
pub struct RecoverRequest {
    pub bolt11: String,
}

#[derive(Serialize)]
struct InvoiceCreated {
    invoice_id: String,
    bolt11: String,
    amount_sats: i64,
    expires_at: i64,
    payment_hash: String,
}

#[derive(Serialize)]
struct InvoiceStatus {
    status: String,
    api_key: Option<String>,
    amount_sats: i64,
    paid_at: Option<i64>,
    created_at: i64,
    expires_at: i64,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(ErrorBody { error: message.into() })).into_response()
}

pub fn derive_api_key(secret: &str, public_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(public_id.as_bytes());
    format!("sk-{}", hex::encode(hasher.finalize()))
}

fn unix(timestamp: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

async fn create_account(db: &Database, secret: &str, public_id: &str) -> Result<i64, sqlx::Error> {
    let username = format!("anon_{}", &public_id[..12]);
    let user = db
        .create_user(NewUser {
            username: Some(&username),
            password_hash: None,
            external_id: None,
            user_type: UserType::Internal,
            is_admin: false,
        })
        .await?;
    let key = derive_api_key(secret, public_id);
    db.create_api_key(NewApiKey {
        key_hash: &hash_api_key(&key),
        name: "lightning",
        user_id: user.id,
        last_four: &get_last_four(&key),
        expires_at: None,
    })
    .await?;
    Ok(user.id)
}

async fn topup_user(state: &AppState, headers: &HeaderMap) -> Result<i64, Response> {
    let token = bearer(headers)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Authorization bearer api key is required for topup"))?;
    let key = state
        .db
        .get_api_key_by_hash(&hash_api_key(token))
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Lookup failed"))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "API key not found"))?;
    Ok(key.user_id)
}

pub async fn create_invoice(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<InvoiceRequest>,
) -> Response {
    let Some(lightning) = state.payments_state.as_ref().and_then(|p| p.lightning_service.clone()) else {
        return error(StatusCode::NOT_FOUND, "Lightning payments not configured on this node");
    };
    if !(MIN_SATS..=MAX_SATS).contains(&body.amount_sats) {
        return error(
            StatusCode::BAD_REQUEST,
            format!("amount_sats must be between {MIN_SATS} and {MAX_SATS}"),
        );
    }

    let _ = state.db.prune_unpaid_anonymous_users().await;
    let secret = match state.db.server_secret().await {
        Ok(s) => s,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Server secret unavailable"),
    };
    let public_id = random_hex();

    let (user_id, created_user) = match body.purpose {
        Purpose::Topup => match topup_user(&state, &headers).await {
            Ok(id) => (id, false),
            Err(response) => return response,
        },
        Purpose::Create => match create_account(&state.db, &secret, &public_id).await {
            Ok(id) => (id, true),
            Err(e) => {
                tracing::error!(error = %e, "Failed to create anonymous account");
                return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create account");
            }
        },
    };

    let memo = format!("Routstr {} {} sats", body.purpose.as_str(), body.amount_sats);
    let instruction = match lightning.create_invoice(user_id, body.amount_sats, &memo, Some(3600)).await {
        Ok(i) => i,
        Err(e) => {
            tracing::error!(error = %e, "Failed to create Lightning invoice");
            if created_user {
                let _ = state.db.delete_user(user_id).await;
            }
            return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create invoice");
        }
    };
    let PaymentInstruction::LightningBolt11 { bolt11, payment_hash, amount_sats, expires_at, .. } = instruction else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Unexpected payment instruction");
    };

    if state
        .db
        .mark_public_invoice(&payment_hash, body.purpose.as_str(), &public_id)
        .await
        .is_err()
    {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to record invoice");
    }

    (
        StatusCode::OK,
        Json(InvoiceCreated {
            invoice_id: public_id,
            bolt11,
            amount_sats,
            expires_at: expires_at.unwrap_or_default(),
            payment_hash,
        }),
    )
        .into_response()
}

async fn status_for(
    state: &AppState,
    row: LightningInvoiceRow,
    public_id: &str,
    purpose: &str,
) -> Response {
    let mut status = row.status.clone();
    if status == "pending" {
        if let Some(expires) = row.expires_at.as_deref().and_then(unix) {
            if chrono::Utc::now().timestamp() > expires {
                let _ = state.db.update_lightning_invoice_status(&row.payment_hash, "expired").await;
                status = "expired".to_string();
            }
        }
    }

    let api_key = if status == "paid" && purpose == "create" {
        state.db.server_secret().await.ok().map(|s| derive_api_key(&s, public_id))
    } else {
        None
    };

    Json(InvoiceStatus {
        status,
        api_key,
        amount_sats: row.amount_sats,
        paid_at: row.paid_at.as_deref().and_then(unix),
        created_at: unix(&row.created_at).unwrap_or_default(),
        expires_at: row.expires_at.as_deref().and_then(unix).unwrap_or_default(),
    })
    .into_response()
}

pub async fn invoice_status(
    State(state): State<Arc<AppState>>,
    Path(invoice_id): Path<String>,
) -> Response {
    let Ok(Some(public)) = state.db.get_public_invoice(&invoice_id).await else {
        return error(StatusCode::NOT_FOUND, "Invoice not found");
    };
    match state.db.get_lightning_invoice_by_hash(&public.payment_hash).await {
        Ok(Some(row)) => status_for(&state, row, &public.public_id, &public.purpose).await,
        _ => error(StatusCode::NOT_FOUND, "Invoice not found"),
    }
}

pub async fn recover_invoice(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RecoverRequest>,
) -> Response {
    let Ok(Some(row)) = state.db.get_invoice_by_bolt11(body.bolt11.trim()).await else {
        return error(StatusCode::NOT_FOUND, "Invoice not found");
    };
    let Ok(Some(public)) = state.db.get_public_invoice_by_hash(&row.payment_hash).await else {
        return error(StatusCode::NOT_FOUND, "Invoice not found");
    };
    status_for(&state, row, &public.public_id, &public.purpose).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn app() -> (Arc<AppState>, axum::Router) {
        use crate::config::{AppConfig, DefaultPricingConfig, PaymentsConfig};
        use crate::metrics::MetricsStore;
        use crate::payments::{balance::BalanceService, pricing::PricingResolver, PaymentsState};

        let db = Arc::new(Database::new("sqlite::memory:").await.unwrap());
        let metrics_store = Arc::new(MetricsStore::new(100));
        let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
        let payments = PaymentsState {
            config: PaymentsConfig::default(),
            balance_service: Arc::new(BalanceService::new(db.clone())),
            lightning_service: None,
            pricing_resolver: Arc::new(PricingResolver::new(db.clone(), DefaultPricingConfig::default())),
            settlement_listener: None,
        };
        let state = Arc::new(AppState {
            config: AppConfig {
                db: db.clone(),
                router,
                auth_config: Default::default(),
                payments_config: None,
                discovery_config: None,
                admin_ui_path: String::new(),
                host: String::new(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(crate::auth::admin::SessionStore::new()),
            db,
            payments_state: Some(Arc::new(payments)),
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
        });
        let app = crate::api::server::create_test_app(state.clone()).await;
        (state, app)
    }

    async fn get(app: &axum::Router, uri: &str, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        use tower::util::ServiceExt;
        let mut req = axum::http::Request::builder().uri(uri);
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let res = app.clone().oneshot(req.body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn paid_invoice_yields_a_working_funded_key() {
        let (state, app) = app().await;
        let secret = state.db.server_secret().await.unwrap();
        let public_id = random_hex();
        let user_id = create_account(&state.db, &secret, &public_id).await.unwrap();
        state
            .db
            .create_lightning_invoice(crate::db::NewLightningInvoice {
                user_id,
                payment_hash: "hash1",
                bolt11: "lnbc1",
                amount_msat: 5_000_000,
                amount_sats: 5000,
                expire_seconds: Some(3600),
            })
            .await
            .unwrap();
        state.db.mark_public_invoice("hash1", "create", &public_id).await.unwrap();

        let uri = format!("/v2/lightning/invoice/{public_id}/status");
        let (code, body) = get(&app, &uri, None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["status"], "pending");
        assert!(body["api_key"].is_null());

        let balance = state.payments_state.as_ref().unwrap().balance_service.clone();
        crate::payments::lightning::handle_settlement(&state.db, &balance, "hash1").await.unwrap();

        let (_, body) = get(&app, &uri, None).await;
        assert_eq!(body["status"], "paid");
        let key = body["api_key"].as_str().unwrap().to_string();
        assert_eq!(key, derive_api_key(&secret, &public_id));

        let (code, info) = get(&app, "/v1/balance/info", Some(&key)).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(info["balance_sats"], 5000);

        let (code, recovered) = {
            use tower::util::ServiceExt;
            let req = axum::http::Request::builder()
                .method("POST")
                .uri("/v2/lightning/recover")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"bolt11":"lnbc1"}"#))
                .unwrap();
            let res = app.clone().oneshot(req).await.unwrap();
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
            (status, serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default())
        };
        assert_eq!(code, StatusCode::OK);
        assert_eq!(recovered["api_key"], key);
    }

    fn nostr_header(keys: &nostr::Keys, path: &str, method: &str) -> String {
        use base64::Engine;
        use nostr::JsonUtil;
        let event = nostr::EventBuilder::new(nostr::Kind::HttpAuth, "")
            .tags(vec![
                nostr::Tag::parse(["u", &format!("https://yalr.example{path}")]).unwrap(),
                nostr::Tag::parse(["method", method]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        format!("Nostr {}", base64::prelude::BASE64_STANDARD.encode(event.as_json()))
    }

    async fn status_with(app: &axum::Router, header: &str, path: &str) -> StatusCode {
        use tower::util::ServiceExt;
        let req = axum::http::Request::builder()
            .uri(path)
            .header("authorization", header)
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn nostr_auth_binds_path_and_honours_the_whitelist() {
        let (state, _) = app().await;
        let allowed = nostr::Keys::generate();
        let stranger = nostr::Keys::generate();

        let mut open = (*state).clone();
        open.config.auth_config.allowed_pubkeys = Default::default();
        let app_open = crate::api::server::create_test_app(Arc::new(open)).await;
        let h = nostr_header(&stranger, "/v1/balance/info", "GET");
        assert_eq!(status_with(&app_open, &h, "/v1/balance/info").await, StatusCode::OK);
        assert_eq!(status_with(&app_open, &h, "/v1/info").await, StatusCode::OK);
        let wrong_path = nostr_header(&stranger, "/v1/models", "GET");
        assert_eq!(status_with(&app_open, &wrong_path, "/v1/balance/info").await, StatusCode::UNAUTHORIZED);

        let mut closed = (*state).clone();
        closed.config.auth_config.allowed_pubkeys = [allowed.public_key().to_string()].into_iter().collect();
        let app_closed = crate::api::server::create_test_app(Arc::new(closed)).await;
        let ok = nostr_header(&allowed, "/v1/balance/info", "GET");
        assert_eq!(status_with(&app_closed, &ok, "/v1/balance/info").await, StatusCode::OK);
        let denied = nostr_header(&stranger, "/v1/balance/info", "GET");
        assert_eq!(status_with(&app_closed, &denied, "/v1/balance/info").await, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn plain_user_sees_and_can_use_only_models_for_sale() {
        use tower::util::ServiceExt;
        let (state, app) = app().await;
        for name in ["sale", "private"] {
            state
                .db
                .create_routing_config(crate::db::NewRoutingConfig {
                    name: name.into(),
                    strategy: "round_robin".into(),
                    health_check_enabled: false,
                    health_check_interval_seconds: 30,
                    health_check_timeout_seconds: 5,
                })
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO model_pricing (model_name, is_advertised) VALUES ('sale', 1)")
            .execute(&state.db.pool)
            .await
            .unwrap();

        let secret = state.db.server_secret().await.unwrap();
        let public_id = random_hex();
        create_account(&state.db, &secret, &public_id).await.unwrap();
        let key = derive_api_key(&secret, &public_id);

        let (code, body) = get(&app, "/v1/models", Some(&key)).await;
        assert_eq!(code, StatusCode::OK);
        let ids: Vec<&str> = body["data"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["sale"]);

        let chat = |model: &str| {
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(format!(
                    r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#
                )))
                .unwrap()
        };
        let denied = app.clone().oneshot(chat("private")).await.unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let bytes = axum::body::to_bytes(denied.into_body(), 1 << 20).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("model_access_denied"));

        let balance = state.payments_state.as_ref().unwrap().balance_service.clone();
        let user = state.db.get_user_by_username(&format!("anon_{}", &public_id[..12])).await.unwrap().unwrap();
        assert_eq!(balance.get_balance(user.id).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn unknown_invoice_is_404_and_invoices_need_lnd() {
        use tower::util::ServiceExt;
        let (_, app) = app().await;
        let (code, _) = get(&app, "/v2/lightning/invoice/nope/status", None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/v2/lightning/invoice")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"amount_sats":1000,"purpose":"topup"}"#))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn derived_key_is_stable_and_scoped() {
        let a = derive_api_key("secret", "abc");
        assert_eq!(a, derive_api_key("secret", "abc"));
        assert_ne!(a, derive_api_key("secret", "abd"));
        assert_ne!(a, derive_api_key("other", "abc"));
        assert!(a.starts_with("sk-"));
        assert_eq!(a.len(), 3 + 64);
    }

    #[tokio::test]
    async fn created_account_accepts_the_derived_key_and_is_not_admin() {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let secret = db.server_secret().await.unwrap();
        let public_id = random_hex();
        let user_id = create_account(&db, &secret, &public_id).await.unwrap();

        let key = derive_api_key(&secret, &public_id);
        let found = db.get_api_key_by_hash(&hash_api_key(&key)).await.unwrap().unwrap();
        assert_eq!(found.user_id, user_id);
        let user = db.get_user_by_id(user_id).await.unwrap().unwrap();
        assert!(!user.is_admin);
    }

    #[test]
    fn amount_bounds_and_purpose_names() {
        assert_eq!(Purpose::Create.as_str(), "create");
        assert_eq!(Purpose::Topup.as_str(), "topup");
        let req: InvoiceRequest = serde_json::from_str(r#"{"amount_sats":500,"purpose":"topup"}"#).unwrap();
        assert_eq!(req.purpose, Purpose::Topup);
        assert!(serde_json::from_str::<InvoiceRequest>(r#"{"amount_sats":5,"purpose":"x"}"#).is_err());
    }

    #[test]
    fn parses_sqlite_timestamps() {
        assert_eq!(unix("1970-01-01 00:01:00"), Some(60));
        assert_eq!(unix("garbage"), None);
    }
}
