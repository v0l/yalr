use std::sync::Arc;

use crate::config::DefaultPricingConfig;
use crate::db::{Database, FiatPrices, NewModelPricing, NewProvider};
use crate::payments::balance::BalanceService;
use crate::payments::biller::{BillingEngine, BillingError, TokenUsage};
use crate::payments::pricing::PricingResolver;

struct Fixture {
    db: Arc<Database>,
    engine: BillingEngine,
    balances: Arc<BalanceService>,
    consumer: i64,
    owner: i64,
}

async fn fixture(currency: &str, fee_ppm: i64) -> Fixture {
    let db = Arc::new(Database::test().await);
    let consumer = db.add_test_user("consumer", false).await.id;
    let owner = db.add_test_user("owner", false).await.id;
    let provider = db
        .create_owned_provider(
            owner,
            NewProvider {
                name: "owner/p",
                slug: "u-owner-p",
                base_url: "https://example.com/v1",
                api_key: None,
                provider_type: None,
            },
        )
        .await
        .unwrap();
    let sats = currency == "sats";
    db.create_model_pricing(NewModelPricing {
        model_name: "acme/m-1",
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: sats.then_some(2_000),
        price_per_1m_output_sats: sats.then_some(4_000),
        price_per_1m_cache_read_sats: None,
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: sats.then_some(1),
        context_window: None,
        max_output_tokens: None,
        owner_user_id: Some(owner),
        provider_id: Some(provider.id),
        currency,
        fiat: if sats {
            FiatPrices::default()
        } else {
            FiatPrices { input: Some(2_000_000), output: Some(4_000_000), request: Some(1000), ..Default::default() }
        },
    })
    .await
    .unwrap();
    let balances = Arc::new(BalanceService::new(db.clone()));
    let resolver = Arc::new(PricingResolver::new(db.clone(), DefaultPricingConfig::default()));
    let engine = BillingEngine::new(balances.clone(), resolver).with_fee_ppm(fee_ppm);
    Fixture { db, engine, balances, consumer, owner }
}

async fn fund(f: &Fixture, currency: &str, amount: i64) {
    if currency == "sats" {
        f.balances.credit(f.consumer, amount, "deposit", "seed").await.unwrap();
    } else {
        f.balances.credit_currency(f.consumer, currency, amount, "deposit", "seed").await.unwrap();
    }
}

async fn balance(f: &Fixture, user: i64, currency: &str) -> i64 {
    f.balances.get_currency_balance(user, currency).await.unwrap()
}

#[tokio::test]
async fn usd_settlement_moves_the_exact_amount_and_writes_both_rows() {
    let f = fixture("usd", 0).await;
    fund(&f, "usd", 10_000_000).await;
    let admission = f.engine.admit(f.consumer, "acme/m-1").await.unwrap();
    let billed = f
        .engine
        .finalize_served(&admission, TokenUsage::plain(1000, 500), Some("owner/p"))
        .await
        .unwrap();
    assert_eq!(billed, 2000 + 2000 + 1000);
    assert_eq!(balance(&f, f.consumer, "usd").await, 10_000_000 - 5000);
    assert_eq!(balance(&f, f.owner, "usd").await, 5000);
    let rows = f.db.list_transactions_for_reference(&admission.request_id).await.unwrap();
    let kinds: Vec<_> = rows.iter().map(|r| (r.user_id, r.transaction_type.as_str(), r.currency.as_str(), r.amount_milli)).collect();
    assert_eq!(
        kinds,
        vec![(f.consumer, "usage", "usd", -5000), (f.owner, "provider_earning", "usd", 5000)]
    );
}

#[tokio::test]
async fn sats_settlement_moves_msats_and_writes_both_rows() {
    let f = fixture("sats", 0).await;
    fund(&f, "sats", 5_000_000).await;
    let admission = f.engine.admit(f.consumer, "acme/m-1").await.unwrap();
    let billed = f
        .engine
        .finalize_served(&admission, TokenUsage::plain(1000, 500), Some("owner/p"))
        .await
        .unwrap();
    assert_eq!(billed, 2000 + 2000 + 1000);
    assert_eq!(balance(&f, f.consumer, "sats").await, 5_000_000 - 5000);
    assert_eq!(balance(&f, f.owner, "sats").await, 5000);
    let rows = f.db.list_transactions_for_reference(&admission.request_id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].amount_msat, rows[0].currency.as_str()), (-5000, "sats"));
    assert_eq!((rows[1].amount_msat, rows[1].transaction_type.as_str()), (5000, "provider_earning"));
}

#[tokio::test]
async fn platform_fee_in_ppm_stays_with_the_platform() {
    let f = fixture("usd", 100_000).await;
    fund(&f, "usd", 10_000_000).await;
    let admission = f.engine.admit(f.consumer, "acme/m-1").await.unwrap();
    f.engine.finalize_served(&admission, TokenUsage::plain(1000, 500), Some("owner/p")).await.unwrap();
    assert_eq!(balance(&f, f.consumer, "usd").await, 10_000_000 - 5000);
    assert_eq!(balance(&f, f.owner, "usd").await, 4500);
}

#[tokio::test]
async fn using_your_own_provider_is_free() {
    let f = fixture("usd", 0).await;
    let admission = f.engine.admit(f.owner, "acme/m-1").await.unwrap();
    let billed = f
        .engine
        .finalize_served(&admission, TokenUsage::plain(1000, 500), Some("owner/p"))
        .await
        .unwrap();
    assert_eq!(billed, 0);
    assert_eq!(balance(&f, f.owner, "usd").await, 0);
    assert!(f.db.list_transactions_for_reference(&admission.request_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_balance_in_another_currency_gets_402() {
    let f = fixture("usd", 0).await;
    fund(&f, "sats", 9_000_000).await;
    match f.engine.admit(f.consumer, "acme/m-1").await {
        Err(BillingError::InsufficientFunds { required, available, currency }) => {
            assert_eq!((required, available, currency.as_str()), (1000, 0, "usd"));
        }
        other => panic!("expected 402, got {other:?}"),
    }
    fund(&f, "usd", 1000).await;
    f.engine.admit(f.consumer, "acme/m-1").await.unwrap();
}

#[tokio::test]
async fn a_settlement_is_all_or_nothing() {
    let f = fixture("usd", 0).await;
    fund(&f, "usd", 10_000).await;
    f.db.execute_raw("DROP TABLE user_currency_balances").await.unwrap();
    let admission = crate::payments::biller::Admission {
        request_id: "req-fail".into(),
        user_id: f.consumer,
        model: "acme/m-1".into(),
        is_free: false,
    };
    let result = f.engine.finalize_served(&admission, TokenUsage::plain(1000, 0), Some("owner/p")).await;
    assert!(result.is_err());
    assert!(f.db.list_transactions_for_reference("req-fail").await.unwrap().is_empty());
}

#[tokio::test]
async fn eur_works_like_any_other_currency() {
    let f = fixture("eur", 0).await;
    fund(&f, "eur", 1_000_000).await;
    let admission = f.engine.admit(f.consumer, "acme/m-1").await.unwrap();
    f.engine.finalize_served(&admission, TokenUsage::plain(1000, 500), Some("owner/p")).await.unwrap();
    assert_eq!(balance(&f, f.owner, "eur").await, 5000);
    assert_eq!(balance(&f, f.owner, "usd").await, 0);
}

#[tokio::test]
async fn a_system_config_routed_to_a_user_provider_bills_the_config_price_and_credits_the_owner() {
    let f = fixture("usd", 100_000).await;
    f.db.create_model_pricing(NewModelPricing {
        model_name: "classify",
        is_advertised: true,
        is_free: false,
        price_per_1m_input_sats: Some(10_000),
        price_per_1m_output_sats: Some(10_000),
        price_per_1m_cache_read_sats: None,
        price_per_1m_cache_write_sats: None,
        price_per_request_sats: Some(0),
        context_window: None,
        max_output_tokens: None,
        owner_user_id: None,
        provider_id: None,
        currency: "sats",
        fiat: FiatPrices::default(),
    })
    .await
    .unwrap();
    fund(&f, "sats", 5_000_000).await;
    let admission = f.engine.admit(f.consumer, "classify").await.unwrap();
    let billed = f
        .engine
        .finalize_served(&admission, TokenUsage::plain(1000, 1000), Some("owner/p"))
        .await
        .unwrap();
    assert_eq!(billed, 20_000);
    assert_eq!(balance(&f, f.consumer, "sats").await, 5_000_000 - 20_000);
    assert_eq!(balance(&f, f.owner, "sats").await, 18_000);
}
