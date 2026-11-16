use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::config::RevolutBusinessConfig;

const TOLERANCE_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, thiserror::Error)]
pub enum RevolutError {
    #[error("revolut request failed: {0}")]
    Http(String),
    #[error("revolut rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("no revolut account is configured for {0}")]
    NoAccount(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Created,
    Failed,
    Awaiting,
    Active,
    Expired,
    Cancelled,
    Processing,
    Processed,
}

impl LinkState {
    pub fn is_dead(self) -> bool {
        matches!(self, Self::Failed | Self::Expired | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PayoutLink {
    pub id: String,
    pub state: LinkState,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub expiry_date: Option<String>,
    #[serde(default)]
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub cancellation_reason: Option<String>,
}

#[derive(Serialize)]
struct CreateLink<'a> {
    counterparty_name: &'a str,
    request_id: &'a str,
    account_id: &'a str,
    amount: f64,
    currency: &'a str,
    reference: &'a str,
    payout_methods: &'a [&'a str],
    expiry_period: &'a str,
    save_counterparty: bool,
}

#[derive(Deserialize)]
struct ApiMessage {
    message: Option<String>,
}

pub struct NewLink<'a> {
    pub payout_id: i64,
    pub recipient_name: &'a str,
    pub currency: &'a str,
    pub amount_milli: i64,
    pub account_id: &'a str,
}

pub fn request_id(payout_id: i64) -> String {
    format!("yalr-payout-{payout_id}")
}

pub fn major_units(amount_milli: i64) -> f64 {
    (amount_milli / 1_000) as f64 / 100.0
}

#[derive(Clone)]
pub struct RevolutBusiness {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    webhook_secret: String,
}

impl RevolutBusiness {
    pub fn new(config: &RevolutBusinessConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
            webhook_secret: config.webhook_secret.clone(),
        }
    }

    pub async fn create_payout_link(&self, new: NewLink<'_>) -> Result<PayoutLink, RevolutError> {
        let request_id = request_id(new.payout_id);
        let reference = format!("YALR payout {}", new.payout_id);
        let body = CreateLink {
            counterparty_name: new.recipient_name,
            request_id: &request_id,
            account_id: new.account_id,
            amount: major_units(new.amount_milli),
            currency: &new.currency.to_ascii_uppercase(),
            reference: &reference,
            payout_methods: &["revolut", "bank_account", "card"],
            expiry_period: "P7D",
            save_counterparty: false,
        };
        let request = self.http.post(format!("{}/payout-links", self.base_url)).json(&body);
        self.send(request).await
    }

    pub async fn get_payout_link(&self, id: &str) -> Result<PayoutLink, RevolutError> {
        self.send(self.http.get(format!("{}/payout-links/{id}", self.base_url))).await
    }

    pub async fn cancel_payout_link(&self, id: &str) -> Result<(), RevolutError> {
        let response = self
            .http
            .post(format!("{}/payout-links/{id}/cancel", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| RevolutError::Http(e.to_string()))?;
        check(response).await.map(|_| ())
    }

    async fn send<T: for<'de> Deserialize<'de>>(&self, request: reqwest::RequestBuilder) -> Result<T, RevolutError> {
        let response = request
            .bearer_auth(&self.api_key)
            .send()
            .await
            .map_err(|e| RevolutError::Http(e.to_string()))?;
        check(response).await?.json().await.map_err(|e| RevolutError::Http(e.to_string()))
    }

    pub fn verify_webhook(&self, signature: &str, timestamp: &str, body: &[u8], now_ms: i64) -> bool {
        if self.webhook_secret.is_empty() {
            return false;
        }
        let Ok(sent) = timestamp.parse::<i64>() else {
            return false;
        };
        if (now_ms - sent).abs() > TOLERANCE_MS {
            return false;
        }
        signature.split(',').any(|candidate| {
            let Some(hex_mac) = candidate.trim().strip_prefix("v1=") else {
                return false;
            };
            let Ok(expected) = hex::decode(hex_mac) else {
                return false;
            };
            let mut mac = Hmac::<Sha256>::new_from_slice(self.webhook_secret.as_bytes()).expect("any key length");
            mac.update(b"v1.");
            mac.update(timestamp.as_bytes());
            mac.update(b".");
            mac.update(body);
            mac.verify_slice(&expected).is_ok()
        })
    }
}

async fn check(response: reqwest::Response) -> Result<reqwest::Response, RevolutError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<ApiMessage>(&text).ok().and_then(|m| m.message).unwrap_or(text);
    Err(RevolutError::Api { status: status.as_u16(), message })
}

#[derive(Deserialize)]
pub struct WebhookEvent {
    pub event: String,
    pub data: WebhookData,
}

#[derive(Deserialize)]
pub struct WebhookData {
    pub id: String,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub new_state: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(base: &str) -> RevolutBusiness {
        RevolutBusiness::new(&RevolutBusinessConfig {
            base_url: base.to_string(),
            api_key: "test-key".into(),
            webhook_secret: "wsk_test".into(),
            account_ids: Default::default(),
        })
    }

    const ACTIVE: &str = r#"{"id":"0e1a8d4b-1d1e-457d-9f10-3e7007a82ea8","state":"active","created_at":"2023-07-11T13:55:54.834963Z","updated_at":"2023-07-11T13:55:55.082655Z","counterparty_name":"John Smith","request_id":"yalr-payout-7","expiry_date":"2023-07-18T13:55:55.082280Z","payout_methods":["revolut","bank_account","card"],"account_id":"85f515e4-588f-4496-a6a5-a7615a193e6b","amount":105.6,"currency":"EUR","url":"https://business.revolut.com/p/uSJVAd5SnP1","reference":"YALR payout 7"}"#;

    #[tokio::test]
    async fn create_link_sends_payout_id_as_request_id_and_exact_amount() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/payout-links"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "request_id": "yalr-payout-7", "amount": 105.6, "currency": "EUR", "account_id": "acc"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_raw(ACTIVE, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
        let link = client(&server.uri())
            .create_payout_link(NewLink {
                payout_id: 7,
                recipient_name: "Jane",
                currency: "eur",
                amount_milli: 10_560_000,
                account_id: "acc",
            })
            .await
            .unwrap();
        assert_eq!(link.state, LinkState::Active);
        assert_eq!(link.url.as_deref(), Some("https://business.revolut.com/p/uSJVAd5SnP1"));
    }

    #[tokio::test]
    async fn processed_link_carries_the_transaction() {
        let server = MockServer::start().await;
        let body = r#"{"id":"L1","state":"processed","transaction_id":"T1","counterparty_id":"C1"}"#;
        Mock::given(method("GET"))
            .and(path("/payout-links/L1"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;
        let link = client(&server.uri()).get_payout_link("L1").await.unwrap();
        assert_eq!(link.state, LinkState::Processed);
        assert_eq!(link.transaction_id.as_deref(), Some("T1"));
    }

    #[tokio::test]
    async fn api_errors_surface_the_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/payout-links/L1/cancel"))
            .respond_with(ResponseTemplate::new(422).set_body_raw(r#"{"message":"already claimed","code":1}"#, "application/json"))
            .mount(&server)
            .await;
        let err = client(&server.uri()).cancel_payout_link("L1").await.unwrap_err();
        assert!(matches!(err, RevolutError::Api { status: 422, ref message } if message == "already claimed"));
    }

    fn sign(secret: &str, ts: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v1.{ts}.").as_bytes());
        mac.update(body);
        format!("v1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn webhook_signature_is_checked_with_timestamp_and_rotation() {
        let c = client("http://x");
        let body = br#"{"event":"PayoutLinkStateChanged"}"#;
        let ts = "1700000000000";
        let good = sign("wsk_test", ts, body);
        assert!(c.verify_webhook(&good, ts, body, 1_700_000_001_000));
        assert!(c.verify_webhook(&format!("v1=00, {good}"), ts, body, 1_700_000_001_000));
        assert!(!c.verify_webhook(&sign("other", ts, body), ts, body, 1_700_000_001_000));
        assert!(!c.verify_webhook(&good, ts, b"tampered", 1_700_000_001_000));
        assert!(!c.verify_webhook(&good, ts, body, 1_700_001_000_000));
    }

    #[test]
    fn amounts_are_whole_cents() {
        assert_eq!(major_units(10_560_000), 105.6);
        assert_eq!(major_units(1_000_000), 10.0);
        assert_eq!(major_units(1_234_999), 12.34);
    }
}
