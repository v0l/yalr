use std::sync::Arc;

use crate::config::DefaultPricingConfig;
use crate::db::{Database, FiatPrices, NewModelPricing, NewProvider};
use crate::payments::balance::BalanceService;
use crate::payments::biller::{BillingEngine, TokenUsage};
use crate::payments::pricing::PricingResolver;
use crate::request_log::FailoverNote;

const MODEL: &str = "acme/m";

struct Fixture {
    db: Arc<Database>,
    engine: BillingEngine,
    balances: Arc<BalanceService>,
    consumer: i64,
    b_owner: i64,
    a_owner: i64,
    currency: &'static str,
}

struct Prices {
    input: i64,
    cache_read: i64,
}

async fn add_provider(db: &Database, name: &str, owner: Option<i64>, currency: &str, prices: Prices) {
    let new = NewProvider {
        name,
        slug: name,
        base_url: "https://example.com/v1",
        api_key: None,
        provider_type: None,
    };
    let provider = match owner {
        Some(owner) => db.create_owned_provider(owner, new).await.unwrap(),
        None => db.create_provider(new).await.unwrap(),
    };
    let sats = currency == "sats";
    let scale = if sats { 1 } else { 1000 };
    db.create_model_pricing(NewModelPricing {
        model_name: MODEL,
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: sats.then_some(prices.input),
        price_per_1m_output_sats: sats.then_some(0),
        price_per_1m_cache_read_sats: sats.then_some(prices.cache_read),
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: sats.then_some(0),
        context_window: None,
        max_output_tokens: None,
        owner_user_id: owner,
        provider_id: Some(provider.id),
        currency,
        fiat: if sats {
            FiatPrices::default()
        } else {
            FiatPrices {
                input: Some(prices.input * scale),
                output: Some(0),
                cache_read: Some(prices.cache_read * scale),
                request: Some(0),
                ..Default::default()
            }
        },
    })
    .await
    .unwrap();
}

async fn fixture(currency: &'static str, a_owned: bool, a_input: i64) -> Fixture {
    let db = Arc::new(Database::test().await);
    let consumer = db.add_test_user("consumer", false).await.id;
    let b_owner = db.add_test_user("b-owner", false).await.id;
    let a_owner = db.add_test_user("a-owner", false).await.id;
    add_provider(&db, "b", Some(b_owner), currency, Prices { input: 2000, cache_read: 200 }).await;
    add_provider(&db, "a", a_owned.then_some(a_owner), currency, Prices { input: a_input, cache_read: 200 }).await;
    let balances = Arc::new(BalanceService::new(db.clone()));
    let resolver = Arc::new(PricingResolver::new(db.clone(), DefaultPricingConfig::default()));
    let engine = BillingEngine::new(balances.clone(), resolver);
    Fixture { db, engine, balances, consumer, b_owner, a_owner, currency }
}

impl Fixture {
    async fn fund(&self, amount: i64) {
        if self.currency == "sats" {
            self.balances.credit(self.consumer, amount, "deposit", "seed").await.unwrap();
        } else {
            self.balances.credit_currency(self.consumer, self.currency, amount, "deposit", "seed").await.unwrap();
        }
    }

    async fn balance(&self, user: i64) -> i64 {
        self.balances.get_currency_balance(user, self.currency).await.unwrap()
    }

    async fn serve(&self, usage: TokenUsage, note: Option<FailoverNote>) -> (i64, String) {
        self.fund(1_000_000).await;
        let admission = self.engine.admit(self.consumer, MODEL).await.unwrap();
        let billed = self.engine.finalize_failover(&admission, usage, Some("b"), note.as_ref()).await.unwrap();
        (billed, admission.request_id)
    }
}

fn note(expected_cached: u32, reason: &str) -> FailoverNote {
    FailoverNote { provider: "a".into(), expected_cached, reason: reason.into() }
}

fn eleven_k() -> TokenUsage {
    TokenUsage::plain(11_000, 0)
}

async fn failover_works_in(currency: &'static str) {
    let f = fixture(currency, true, 2000).await;
    let (billed, request_id) = f.serve(eleven_k(), Some(note(10_000, "503"))).await;
    assert_eq!(billed, 4000);
    assert_eq!(f.balance(f.consumer).await, 1_000_000 - 4000);
    assert_eq!(f.balance(f.b_owner).await, 22_000);
    assert_eq!(f.balance(f.a_owner).await, -18_000);
    let rows = f.db.list_transactions_for_reference(&request_id).await.unwrap();
    let kinds: Vec<_> = rows.iter().map(|r| r.transaction_type.as_str()).collect();
    assert_eq!(kinds, vec!["usage", "provider_earning", "failover_penalty"]);
    let totals = f.db.failover_penalty_totals(0).await.unwrap();
    assert_eq!(totals.len(), 1);
    assert_eq!(
        (totals[0].failed_provider.as_str(), totals[0].currency.as_str(), totals[0].penalty_milli, totals[0].platform_milli),
        ("a", currency, 18_000, 0)
    );
}

#[tokio::test]
async fn a_failed_user_provider_pays_the_prefill_in_sats() {
    failover_works_in("sats").await;
}

#[tokio::test]
async fn a_failed_user_provider_pays_the_prefill_in_usd() {
    failover_works_in("usd").await;
}

#[tokio::test]
async fn a_failed_platform_provider_is_absorbed_by_the_platform() {
    let f = fixture("sats", false, 2000).await;
    let (billed, request_id) = f.serve(eleven_k(), Some(note(10_000, "503"))).await;
    assert_eq!(billed, 4000);
    assert_eq!(f.balance(f.b_owner).await, 22_000);
    assert_eq!(f.balance(f.a_owner).await, 0);
    let rows = f.db.list_transactions_for_reference(&request_id).await.unwrap();
    assert!(rows.iter().all(|r| r.transaction_type != "failover_penalty"));
    let totals = f.db.failover_penalty_totals(0).await.unwrap();
    assert_eq!((totals[0].penalty_milli, totals[0].platform_milli), (0, 18_000));
}

#[tokio::test]
async fn the_penalty_is_capped_at_the_failed_providers_own_price() {
    let f = fixture("sats", true, 1000).await;
    let (billed, _) = f.serve(eleven_k(), Some(note(10_000, "503"))).await;
    assert_eq!(billed, 4000);
    assert_eq!(f.balance(f.a_owner).await, -(10_000 * 1000 / 1000 - 10_000 * 200 / 1000));
    let totals = f.db.failover_penalty_totals(0).await.unwrap();
    assert_eq!((totals[0].penalty_milli, totals[0].platform_milli), (8000, 10_000));
}

#[tokio::test]
async fn without_a_failover_note_the_consumer_pays_everything_and_nobody_is_penalised() {
    let f = fixture("sats", true, 2000).await;
    let (billed, request_id) = f.serve(eleven_k(), None).await;
    assert_eq!(billed, 22_000);
    assert_eq!(f.balance(f.a_owner).await, 0);
    assert_eq!(f.db.list_transactions_for_reference(&request_id).await.unwrap().len(), 2);
    assert!(f.db.failover_penalty_totals(0).await.unwrap().is_empty());
}

#[tokio::test]
async fn only_the_changed_prefix_is_credited() {
    let f = fixture("sats", true, 2000).await;
    let (billed, _) = f.serve(eleven_k(), Some(note(2_500, "503"))).await;
    assert_eq!(billed, 22_000 - (2_500 * 2000 - 2_500 * 200) / 1000);
}

#[tokio::test]
async fn reported_cache_hits_beyond_the_expected_prefix_are_kept() {
    let usage = TokenUsage { prompt_tokens: 11_000, completion_tokens: 0, cached_tokens: 10_500, cache_write_tokens: 400 };
    let adjusted = usage.with_expected_cached(10_000);
    assert_eq!((adjusted.cached_tokens, adjusted.cache_write_tokens), (10_500, 400));
    let cold = TokenUsage { prompt_tokens: 11_000, completion_tokens: 0, cached_tokens: 0, cache_write_tokens: 10_500 };
    let adjusted = cold.with_expected_cached(10_000);
    assert_eq!((adjusted.cached_tokens, adjusted.cache_write_tokens), (10_000, 1_000));
}

#[tokio::test]
async fn the_expected_prefix_never_exceeds_the_prompt() {
    let usage = TokenUsage::plain(500, 0).with_expected_cached(10_000);
    assert_eq!(usage.cached_tokens, 500);
}

#[tokio::test]
async fn a_negative_balance_blocks_withdrawals() {
    let f = fixture("sats", true, 2000).await;
    f.serve(eleven_k(), Some(note(10_000, "503"))).await;
    assert_eq!(f.balance(f.a_owner).await, -18_000);
    assert_eq!(f.db.withdrawable(f.a_owner, "sats", 0).await.unwrap(), 0);
    assert_eq!(f.db.withdrawable(f.b_owner, "sats", 0).await.unwrap(), 22_000);
}
