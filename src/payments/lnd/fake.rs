use super::{InvoiceEvent, InvoiceState, LightningNode, NewInvoice, PaidInvoice};
use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use std::collections::BTreeMap;
use std::sync::Mutex;
use futures::channel::mpsc;

#[derive(Default)]
pub struct FakeState {
    pub invoices: BTreeMap<String, (u64, InvoiceEvent)>,
    pub next_settle: u64,
    pub subscribed_after: Vec<u64>,
    pub listeners: Vec<mpsc::UnboundedSender<Result<InvoiceEvent, String>>>,
    pub paid: Vec<String>,
    pub fail_add: bool,
    pub fail_pay: bool,
    created: u64,
}

#[derive(Default)]
pub struct FakeLnd {
    pub state: Mutex<FakeState>,
}

impl FakeLnd {
    pub fn hash_of(&self, n: u64) -> String {
        format!("{n:064x}")
    }

    pub fn last_hash(&self) -> String {
        let s = self.state.lock().unwrap();
        self.hash_of(s.created)
    }

    fn update(&self, hash: &str, state: InvoiceState, amt_paid_msat: Option<i64>, live: bool) -> InvoiceEvent {
        let mut s = self.state.lock().unwrap();
        let settle_index = if state == InvoiceState::Settled {
            s.next_settle += 1;
            s.next_settle
        } else {
            0
        };
        let (amount, event) = s.invoices.get_mut(hash).expect("unknown fake invoice");
        event.state = state;
        event.settle_index = settle_index;
        event.amt_paid_msat = amt_paid_msat.unwrap_or(*amount as i64);
        let event = event.clone();
        if live {
            s.listeners.retain(|l| l.unbounded_send(Ok(event.clone())).is_ok());
        }
        event
    }

    pub fn settle(&self, hash: &str) -> InvoiceEvent {
        self.update(hash, InvoiceState::Settled, None, true)
    }

    pub fn settle_for(&self, hash: &str, amt_paid_msat: i64) -> InvoiceEvent {
        self.update(hash, InvoiceState::Settled, Some(amt_paid_msat), true)
    }

    pub fn settle_while_offline(&self, hash: &str) -> InvoiceEvent {
        self.update(hash, InvoiceState::Settled, None, false)
    }

    pub fn cancel(&self, hash: &str) -> InvoiceEvent {
        self.update(hash, InvoiceState::Canceled, Some(0), true)
    }

    pub fn replay(&self, event: &InvoiceEvent) {
        self.state.lock().unwrap().listeners.retain(|l| l.unbounded_send(Ok(event.clone())).is_ok());
    }

    pub fn drop_streams(&self) {
        self.state.lock().unwrap().listeners.clear();
    }

    pub fn subscriptions(&self) -> Vec<u64> {
        self.state.lock().unwrap().subscribed_after.clone()
    }
}

#[async_trait]
impl LightningNode for FakeLnd {
    async fn add_invoice(&self, amount_msat: u64, _memo: &str, _expiry_secs: u32) -> Result<NewInvoice, String> {
        let mut s = self.state.lock().unwrap();
        if s.fail_add {
            return Err("fake lnd is down".into());
        }
        s.created += 1;
        let hash = self.hash_of(s.created);
        let event = InvoiceEvent { payment_hash: hash.clone(), state: InvoiceState::Open, settle_index: 0, amt_paid_msat: 0 };
        s.invoices.insert(hash.clone(), (amount_msat, event));
        Ok(NewInvoice { bolt11: format!("lnbcrt{}fake{}", amount_msat / 1000, s.created), payment_hash: hash })
    }

    async fn lookup_invoice(&self, payment_hash: &str) -> Result<Option<InvoiceEvent>, String> {
        Ok(self.state.lock().unwrap().invoices.get(payment_hash).map(|(_, e)| e.clone()))
    }

    async fn subscribe(&self, after_settle_index: u64) -> Result<BoxStream<'static, Result<InvoiceEvent, String>>, String> {
        let (tx, rx) = mpsc::unbounded();
        let mut s = self.state.lock().unwrap();
        s.subscribed_after.push(after_settle_index);
        let mut backlog: Vec<InvoiceEvent> = s
            .invoices
            .values()
            .map(|(_, e)| e.clone())
            .filter(|e| e.state == InvoiceState::Settled && e.settle_index > after_settle_index)
            .collect();
        backlog.sort_by_key(|e| e.settle_index);
        for event in backlog {
            let _ = tx.unbounded_send(Ok(event));
        }
        s.listeners.push(tx);
        Ok(rx.boxed())
    }

    async fn pay_invoice(&self, bolt11: &str) -> Result<PaidInvoice, String> {
        let mut s = self.state.lock().unwrap();
        if s.fail_pay {
            return Err("no route".into());
        }
        s.paid.push(bolt11.to_string());
        Ok(PaidInvoice { payment_hash: format!("paid_{}", s.paid.len()), fee_msat: 0 })
    }
}
