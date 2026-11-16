use std::str::FromStr;

use async_trait::async_trait;
use lightning_invoice::Bolt11Invoice;
use serde::Deserialize;

pub struct Paid {
    pub payment_hash: String,
    pub fee_msat: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LightningPayoutError {
    #[error("destination is not a BOLT11 invoice or a Lightning address")]
    BadDestination,
    #[error("invoice has no amount, ask for one with the exact payout amount")]
    NoAmount,
    #[error("invoice amount {invoice_msat} msat does not match the payout of {expected_msat} msat")]
    AmountMismatch { invoice_msat: u64, expected_msat: u64 },
    #[error("invoice has expired")]
    Expired,
    #[error("lightning address lookup failed: {0}")]
    Lnurl(String),
    #[error("payment failed: {0}")]
    Payment(String),
}

#[async_trait]
pub trait InvoicePayer: Send + Sync {
    async fn pay(&self, bolt11: &str) -> Result<Paid, String>;
}

pub struct ParsedInvoice {
    pub payment_hash: String,
}

pub fn check_invoice(bolt11: &str, expected_msat: u64) -> Result<ParsedInvoice, LightningPayoutError> {
    let invoice = Bolt11Invoice::from_str(bolt11.trim()).map_err(|_| LightningPayoutError::BadDestination)?;
    let invoice_msat = invoice.amount_milli_satoshis().ok_or(LightningPayoutError::NoAmount)?;
    if invoice_msat != expected_msat {
        return Err(LightningPayoutError::AmountMismatch { invoice_msat, expected_msat });
    }
    if invoice.is_expired() {
        return Err(LightningPayoutError::Expired);
    }
    Ok(ParsedInvoice { payment_hash: hex::encode(invoice.payment_hash().as_ref() as &[u8]) })
}

pub fn is_lightning_address(destination: &str) -> bool {
    let Some((name, host)) = destination.trim().split_once('@') else {
        return false;
    };
    !name.is_empty()
        && !host.is_empty()
        && host.contains('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

pub fn is_bolt11(destination: &str) -> bool {
    let lower = destination.trim().to_ascii_lowercase();
    ["lnbc", "lntb", "lnbcrt", "lntbs"].iter().any(|p| lower.starts_with(p))
}

pub fn validate_destination(destination: &str) -> Result<(), LightningPayoutError> {
    if is_bolt11(destination) || is_lightning_address(destination) {
        Ok(())
    } else {
        Err(LightningPayoutError::BadDestination)
    }
}

#[derive(Deserialize)]
struct PayParams {
    callback: String,
    #[serde(rename = "minSendable")]
    min_sendable: u64,
    #[serde(rename = "maxSendable")]
    max_sendable: u64,
}

#[derive(Deserialize)]
struct PayResponse {
    pr: Option<String>,
    reason: Option<String>,
}

pub async fn invoice_from_address(
    http: &reqwest::Client,
    address: &str,
    amount_msat: u64,
    scheme: &str,
    allow_private: bool,
) -> Result<String, LightningPayoutError> {
    let lnurl_err = |e: &dyn ToString| LightningPayoutError::Lnurl(e.to_string());
    let (name, host) = address.trim().split_once('@').ok_or(LightningPayoutError::BadDestination)?;
    let params_url = format!("{scheme}://{host}/.well-known/lnurlp/{name}");
    if !allow_private {
        crate::ssrf::check_public_url(&params_url).await.map_err(|e| lnurl_err(&e))?;
    }
    let params: PayParams = http
        .get(&params_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| lnurl_err(&e))?
        .json()
        .await
        .map_err(|e| lnurl_err(&e))?;
    if amount_msat < params.min_sendable || amount_msat > params.max_sendable {
        return Err(LightningPayoutError::Lnurl("amount is outside what the address accepts".into()));
    }
    let mut callback = url::Url::parse(&params.callback).map_err(|e| lnurl_err(&e))?;
    callback.query_pairs_mut().append_pair("amount", &amount_msat.to_string());
    if !allow_private {
        crate::ssrf::check_public_url(callback.as_str()).await.map_err(|e| lnurl_err(&e))?;
    }
    let reply: PayResponse = http
        .get(callback)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| lnurl_err(&e))?
        .json()
        .await
        .map_err(|e| lnurl_err(&e))?;
    reply.pr.ok_or_else(|| LightningPayoutError::Lnurl(reply.reason.unwrap_or_else(|| "no invoice returned".into())))
}

pub struct Resolved {
    pub bolt11: String,
    pub payment_hash: String,
}

pub async fn resolve_destination(
    http: &reqwest::Client,
    destination: &str,
    amount_msat: u64,
) -> Result<Resolved, LightningPayoutError> {
    let bolt11 = if is_bolt11(destination) {
        destination.trim().to_string()
    } else {
        invoice_from_address(http, destination, amount_msat, "https", false).await?
    };
    let parsed = check_invoice(&bolt11, amount_msat)?;
    Ok(Resolved { bolt11, payment_hash: parsed.payment_hash })
}

pub async fn pay_destination(
    payer: &dyn InvoicePayer,
    http: &reqwest::Client,
    destination: &str,
    amount_msat: u64,
) -> Result<Paid, LightningPayoutError> {
    let resolved = resolve_destination(http, destination, amount_msat).await?;
    payer.pay(&resolved.bolt11).await.map_err(LightningPayoutError::Payment)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::payouts::test_invoice::invoice;

    #[test]
    fn destinations_are_recognised() {
        assert!(is_lightning_address("jane@getalby.com"));
        assert!(!is_lightning_address("jane@localhost"));
        assert!(!is_lightning_address("not an address"));
        assert!(validate_destination(&invoice(Some(1000), 1)).is_ok());
        assert_eq!(validate_destination("hello"), Err(LightningPayoutError::BadDestination));
    }

    #[test]
    fn invoice_amount_must_match_the_payout_exactly() {
        let bolt11 = invoice(Some(250_000_000), 1);
        assert!(check_invoice(&bolt11, 250_000_000).is_ok());
        assert_eq!(
            check_invoice(&bolt11, 1_000).err(),
            Some(LightningPayoutError::AmountMismatch { invoice_msat: 250_000_000, expected_msat: 1_000 })
        );
        assert_eq!(check_invoice(&invoice(None, 1), 1_000).err(), Some(LightningPayoutError::NoAmount));
        assert_eq!(check_invoice("garbage", 1).err(), Some(LightningPayoutError::BadDestination));
    }

    #[tokio::test]
    async fn lightning_address_resolves_through_lnurl_pay() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let params = format!(
            r#"{{"callback":"{}/cb","minSendable":1000,"maxSendable":1000000000,"tag":"payRequest"}}"#,
            server.uri()
        );
        Mock::given(method("GET"))
            .and(path("/.well-known/lnurlp/jane"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(params, "application/json"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cb"))
            .and(query_param("amount", "5000000"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"pr":"lnbc1fake"}"#, "application/json"))
            .mount(&server)
            .await;
        let host = server.uri().replace("http://", "");
        let client = reqwest::Client::new();
        let address = format!("jane@{host}");
        let pr = invoice_from_address(&client, &address, 5_000_000, "http", true).await.unwrap();
        assert_eq!(pr, "lnbc1fake");
        assert!(invoice_from_address(&client, &address, 2_000_000_000, "http", true).await.is_err());
        assert!(invoice_from_address(&client, &address, 5_000_000, "http", false).await.is_err());
    }

    struct MockLnd(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl InvoicePayer for MockLnd {
        async fn pay(&self, bolt11: &str) -> Result<Paid, String> {
            self.0.lock().unwrap().push(bolt11.to_string());
            Ok(Paid { payment_hash: "h".into(), fee_msat: 5 })
        }
    }

    #[tokio::test]
    async fn a_mismatched_invoice_never_reaches_the_node() {
        let lnd = MockLnd(Default::default());
        let client = reqwest::Client::new();
        let err = pay_destination(&lnd, &client, &invoice(Some(250_000_000), 1), 1_000).await.err();
        assert!(matches!(err, Some(LightningPayoutError::AmountMismatch { .. })));
        assert!(lnd.0.lock().unwrap().is_empty());
    }
}
