use super::setup_test_db;
use crate::db::payouts::{NewPayout, PayoutError, PayoutRules, COMPLETED, FAILED, HELD, LIGHTNING, REQUESTED, REVOLUT_LINK};
use crate::db::Database;

fn rules() -> PayoutRules {
    PayoutRules {
        min_amount: 1_000,
        daily_cap: 100_000,
        hold_days: 7,
        approval_threshold: 50_000,
        fee_ppm: 0,
        fee_flat: 0,
    }
}

async fn earn(db: &Database, user: i64, currency: &str, amount: i64, age_days: i64) {
    db.credit_currency(user, currency, amount, "provider_earning", "e", 0).await.unwrap();
    db.execute_raw(&format!(
        "UPDATE balance_transactions SET created_at = created_at - INTERVAL {age_days} DAY \
         WHERE user_id = {user} AND transaction_type = 'provider_earning' ORDER BY id DESC LIMIT 1"
    ))
    .await
    .unwrap();
}

fn new<'a>(user: i64, currency: &'a str, amount: i64, method: &'a str) -> NewPayout<'a> {
    NewPayout { user_id: user, currency, amount, method, destination: "dest", external_id: None }
}

async fn approved_first(db: &Database, user: i64, currency: &str) {
    earn(db, user, currency, 10_000, 30).await;
    let p = db.request_payout(new(user, currency, 1_000, LIGHTNING), &rules()).await.unwrap();
    db.approve_payout(p.id).await.unwrap();
    db.mark_payout_sent(p.id, &format!("h{}", p.id)).await.unwrap();
    db.complete_payout(p.id).await.unwrap();
}

#[tokio::test]
async fn deposits_are_not_withdrawable() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    db.credit_currency(u.id, "eur", 500_000, "deposit", "d", 500_000).await.unwrap();
    assert_eq!(db.withdrawable(u.id, "eur", 7).await.unwrap(), 0);
    let err = db.request_payout(new(u.id, "eur", 1_000, REVOLUT_LINK), &rules()).await.unwrap_err();
    assert_eq!(err, PayoutError::NotWithdrawable { withdrawable: 0 });
}

#[tokio::test]
async fn new_earnings_wait_out_the_hold_period() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    earn(&db, u.id, "usd", 5_000, 2).await;
    assert_eq!(db.withdrawable(u.id, "usd", 7).await.unwrap(), 0);
    earn(&db, u.id, "usd", 3_000, 8).await;
    assert_eq!(db.withdrawable(u.id, "usd", 7).await.unwrap(), 3_000);
}

#[tokio::test]
async fn withdrawable_is_earned_not_balance_with_topups() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    db.credit_currency(u.id, "eur", 90_000, "deposit", "d", 90_000).await.unwrap();
    earn(&db, u.id, "eur", 10_000, 10).await;
    assert_eq!(db.withdrawable(u.id, "eur", 7).await.unwrap(), 10_000);
}

#[tokio::test]
async fn first_payout_needs_approval_and_holds_nothing_until_approved() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    earn(&db, u.id, "eur", 10_000, 10).await;
    let p = db.request_payout(new(u.id, "eur", 4_000, REVOLUT_LINK), &rules()).await.unwrap();
    assert_eq!(p.status, REQUESTED);
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 10_000);
    assert_eq!(db.withdrawable(u.id, "eur", 7).await.unwrap(), 6_000);
    db.approve_payout(p.id).await.unwrap();
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 6_000);
    assert_eq!(db.approve_payout(p.id).await.unwrap_err(), PayoutError::WrongState);
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 6_000);
}

#[tokio::test]
async fn later_small_payout_is_held_at_once_and_large_one_needs_approval() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "eur").await;
    earn(&db, u.id, "eur", 200_000, 10).await;
    let small = db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &rules()).await.unwrap();
    assert_eq!(small.status, HELD);
    let large = db.request_payout(new(u.id, "eur", 60_000, REVOLUT_LINK), &rules()).await.unwrap();
    assert_eq!(large.status, REQUESTED);
}

#[tokio::test]
async fn failure_returns_the_hold_once() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "eur").await;
    let before = db.get_currency_balance(u.id, "eur").await.unwrap();
    let p = db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &rules()).await.unwrap();
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), before - 2_000);
    db.mark_payout_sent(p.id, "link1").await.unwrap();
    db.fail_payout(p.id, "expired").await.unwrap();
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), before);
    assert_eq!(db.fail_payout(p.id, "again").await.unwrap_err(), PayoutError::WrongState);
    assert_eq!(db.cancel_payout(p.id).await.unwrap_err(), PayoutError::WrongState);
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), before);
    assert_eq!(db.get_payout(p.id).await.unwrap().unwrap().status, FAILED);
}

#[tokio::test]
async fn completion_finalises_and_cannot_be_failed_afterwards() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "eur").await;
    let p = db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &rules()).await.unwrap();
    db.mark_payout_sent(p.id, "link2").await.unwrap();
    let after_hold = db.get_currency_balance(u.id, "eur").await.unwrap();
    db.complete_payout(p.id).await.unwrap();
    assert_eq!(db.complete_payout(p.id).await.unwrap_err(), PayoutError::WrongState);
    assert_eq!(db.fail_payout(p.id, "late").await.unwrap_err(), PayoutError::WrongState);
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), after_hold);
    assert_eq!(db.get_payout(p.id).await.unwrap().unwrap().status, COMPLETED);
}

#[tokio::test]
async fn racing_completions_and_failures_apply_one_outcome() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "eur").await;
    let p = db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &rules()).await.unwrap();
    db.mark_payout_sent(p.id, "link3").await.unwrap();
    let before = db.get_currency_balance(u.id, "eur").await.unwrap();
    let (a, b, c) = tokio::join!(db.complete_payout(p.id), db.fail_payout(p.id, "x"), db.complete_payout(p.id));
    let wins = [a.is_ok(), b.is_ok(), c.is_ok()].into_iter().filter(|ok| *ok).count();
    assert_eq!(wins, 1);
    let status = db.get_payout(p.id).await.unwrap().unwrap().status;
    let expected = if status == FAILED { before + 2_000 } else { before };
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), expected);
}

#[tokio::test]
async fn same_external_id_cannot_be_used_twice() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    earn(&db, u.id, "sats", 50_000_000, 10).await;
    let rules = PayoutRules { daily_cap: i64::MAX / 2, ..rules() };
    let mut first = new(u.id, "sats", 6_000_000, LIGHTNING);
    first.external_id = Some("hash");
    let ok = db.request_payout(first, &PayoutRules { approval_threshold: 0, ..rules.clone() }).await.unwrap();
    assert_eq!(ok.status, REQUESTED);
    let mut dup = new(u.id, "sats", 6_000_000, LIGHTNING);
    dup.external_id = Some("hash");
    assert_eq!(db.request_payout(dup, &rules).await.unwrap_err(), PayoutError::Duplicate);
}

#[tokio::test]
async fn minimum_and_daily_cap_apply() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "eur").await;
    earn(&db, u.id, "eur", 500_000, 10).await;
    assert_eq!(
        db.request_payout(new(u.id, "eur", 999, REVOLUT_LINK), &rules()).await.unwrap_err(),
        PayoutError::BelowMinimum { min: 1_000 }
    );
    let capped = PayoutRules { daily_cap: 3_500, ..rules() };
    db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &capped).await.unwrap();
    assert_eq!(
        db.request_payout(new(u.id, "eur", 2_000, REVOLUT_LINK), &capped).await.unwrap_err(),
        PayoutError::OverDailyCap { remaining: 500 }
    );
}

#[tokio::test]
async fn rejected_request_returns_nothing_because_nothing_was_held() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    earn(&db, u.id, "eur", 10_000, 10).await;
    let p = db.request_payout(new(u.id, "eur", 4_000, REVOLUT_LINK), &rules()).await.unwrap();
    db.reject_payout(p.id, "no").await.unwrap();
    assert_eq!(db.get_currency_balance(u.id, "eur").await.unwrap(), 10_000);
    assert_eq!(db.withdrawable(u.id, "eur", 7).await.unwrap(), 10_000);
}

#[tokio::test]
async fn sats_use_the_msat_column_and_fee_comes_off_the_recipient() {
    let db = setup_test_db().await;
    let u = db.add_test_user("u", false).await;
    approved_first(&db, u.id, "sats").await;
    let before = db.get_currency_balance(u.id, "sats").await.unwrap();
    let with_fee = PayoutRules { fee_ppm: 10_000, fee_flat: 1_000, ..rules() };
    let p = db.request_payout(new(u.id, "sats", 2_000, LIGHTNING), &with_fee).await.unwrap();
    assert_eq!(p.fee, 1_020);
    assert_eq!(db.get_currency_balance(u.id, "sats").await.unwrap(), before - 2_000);
    assert_eq!(p.status, HELD);
}
