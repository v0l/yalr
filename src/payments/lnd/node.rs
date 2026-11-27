use async_trait::async_trait;
use futures::stream::BoxStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceState {
    Open,
    Accepted,
    Settled,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceEvent {
    pub payment_hash: String,
    pub state: InvoiceState,
    pub settle_index: u64,
    pub amt_paid_msat: i64,
}

#[derive(Debug, Clone)]
pub struct NewInvoice {
    pub bolt11: String,
    pub payment_hash: String,
}

#[derive(Debug, Clone)]
pub struct PaidInvoice {
    pub payment_hash: String,
    pub fee_msat: u64,
}

#[async_trait]
pub trait LightningNode: Send + Sync {
    async fn add_invoice(&self, amount_msat: u64, memo: &str, expiry_secs: u32) -> Result<NewInvoice, String>;

    async fn lookup_invoice(&self, payment_hash: &str) -> Result<Option<InvoiceEvent>, String>;

    async fn subscribe(&self, after_settle_index: u64) -> Result<BoxStream<'static, Result<InvoiceEvent, String>>, String>;

    async fn pay_invoice(&self, bolt11: &str) -> Result<PaidInvoice, String>;
}

pub struct LndNode {
    inner: payments_rs::lightning::LndNode,
}

impl LndNode {
    pub async fn connect(settings: &crate::config::LndSettings) -> Result<Self, String> {
        payments_rs::lightning::setup_crypto_provider();
        let inner = payments_rs::lightning::LndNode::new(
            &settings.url,
            std::path::Path::new(&settings.tls_cert_path),
            std::path::Path::new(&settings.macaroon_path),
        )
        .await
        .map_err(|e| format!("could not connect to LND at {}: {e}", settings.url))?;
        Ok(Self { inner })
    }
}

fn event(invoice: fedimint_tonic_lnd::lnrpc::Invoice) -> InvoiceEvent {
    use fedimint_tonic_lnd::lnrpc::invoice::InvoiceState as Lnd;
    let state = match Lnd::try_from(invoice.state) {
        Ok(Lnd::Settled) => InvoiceState::Settled,
        Ok(Lnd::Canceled) => InvoiceState::Canceled,
        Ok(Lnd::Accepted) => InvoiceState::Accepted,
        _ => InvoiceState::Open,
    };
    InvoiceEvent { payment_hash: hex::encode(&invoice.r_hash), state, settle_index: invoice.settle_index, amt_paid_msat: invoice.amt_paid_msat }
}

#[async_trait]
impl LightningNode for LndNode {
    async fn add_invoice(&self, amount_msat: u64, memo: &str, expiry_secs: u32) -> Result<NewInvoice, String> {
        use payments_rs::lightning::LightningNode as _;
        let created = self
            .inner
            .add_invoice(payments_rs::lightning::AddInvoiceRequest { amount: amount_msat, memo: Some(memo.to_string()), expire: Some(expiry_secs) })
            .await
            .map_err(|e| e.to_string())?;
        Ok(NewInvoice { bolt11: created.pr(), payment_hash: created.payment_hash() })
    }

    async fn lookup_invoice(&self, payment_hash: &str) -> Result<Option<InvoiceEvent>, String> {
        let r_hash = hex::decode(payment_hash).map_err(|e| e.to_string())?;
        let mut client = self.inner.client();
        match client.lightning().lookup_invoice(fedimint_tonic_lnd::lnrpc::PaymentHash { r_hash, ..Default::default() }).await {
            Ok(found) => Ok(Some(event(found.into_inner()))),
            Err(status) if status.code() == fedimint_tonic_lnd::tonic::Code::NotFound => Ok(None),
            Err(status) if status.message().contains("unable to locate invoice") => Ok(None),
            Err(status) => Err(status.to_string()),
        }
    }

    async fn subscribe(&self, after_settle_index: u64) -> Result<BoxStream<'static, Result<InvoiceEvent, String>>, String> {
        use futures::StreamExt;
        let mut client = self.inner.client();
        let stream = client
            .lightning()
            .subscribe_invoices(fedimint_tonic_lnd::lnrpc::InvoiceSubscription { add_index: 0, settle_index: after_settle_index })
            .await
            .map_err(|e| e.to_string())?
            .into_inner();
        Ok(stream.map(|item| item.map(event).map_err(|e| e.to_string())).boxed())
    }

    async fn pay_invoice(&self, bolt11: &str) -> Result<PaidInvoice, String> {
        use payments_rs::lightning::LightningNode as _;
        let paid = self
            .inner
            .pay_invoice(payments_rs::lightning::PayInvoiceRequest { invoice: bolt11.to_string(), timeout_seconds: Some(120) })
            .await
            .map_err(|e| e.to_string())?;
        Ok(PaidInvoice { payment_hash: paid.payment_hash, fee_msat: paid.fee_msat })
    }
}
