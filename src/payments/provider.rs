use crate::db::gateway_payments::GatewayPayment;
use async_trait::async_trait;
use axum::http::{HeaderMap, StatusCode};

#[derive(Debug)]
pub enum PaymentError {
    Invalid(String),
    Rejected(String),
    Upstream(String),
    Database(sqlx::Error),
}

impl std::fmt::Display for PaymentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaymentError::Invalid(m) | PaymentError::Rejected(m) | PaymentError::Upstream(m) => f.write_str(m),
            PaymentError::Database(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for PaymentError {}

impl From<sqlx::Error> for PaymentError {
    fn from(e: sqlx::Error) -> Self {
        PaymentError::Database(e)
    }
}

pub struct TopupRequest<'a> {
    pub user_id: i64,
    pub kind: &'a str,
    pub purpose: Option<&'a str>,
    pub currency: &'a str,
    pub amount_milli: i64,
    pub lightning: bool,
    pub email: Option<&'a str>,
    pub public_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Gateway,
    Lnd,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Gateway => "gateway",
            ProviderKind::Lnd => "lnd",
        }
    }
}

pub const CARDS_NEED_GATEWAY: &str = "Card payments need the payment gateway (payments.gateway); this node takes Lightning through its own LND";

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    fn mode(&self) -> &'static str;

    fn currencies(&self) -> Vec<String>;

    fn amount_range(&self) -> (i64, i64);

    fn currency_allowed(&self, currency: &str) -> bool {
        self.currencies().iter().any(|c| c == currency)
    }

    fn check_amount(&self, currency: &str, amount: f64) -> Result<i64, PaymentError>;

    async fn create_topup(&self, request: TopupRequest<'_>) -> Result<GatewayPayment, PaymentError>;

    async fn refresh(&self, payment: &GatewayPayment) -> Result<GatewayPayment, PaymentError>;

    async fn handle_webhook(&self, _headers: &HeaderMap, _body: &[u8]) -> StatusCode {
        StatusCode::NOT_FOUND
    }

    async fn refund(&self, user_id: i64, amount_msat: i64, invoice: &str) -> Result<String, String>;

    async fn btc_usd(&self) -> Option<f64> {
        None
    }
}
