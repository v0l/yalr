use sqlx::migrate::Migrator;
use sqlx::{Connection, Executor, MySqlConnection};
use std::borrow::Cow;

const CUTOVER_VERSION: i64 = 20261119000001;

fn server() -> String {
    std::env::var("YALR_TEST_DATABASE_URL").unwrap_or_else(|_| "mysql://root:root@127.0.0.1:3307".to_string())
}

async fn migrate(conn: &mut MySqlConnection, upto_before_cutover: bool) {
    let mut migrator: Migrator = sqlx::migrate!("./migrations");
    if upto_before_cutover {
        migrator.migrations = Cow::Owned(migrator.migrations.iter().filter(|m| m.version < CUTOVER_VERSION).cloned().collect());
    }
    migrator.run_direct(conn).await.unwrap();
}

async fn count(conn: &mut MySqlConnection, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&mut *conn).await.unwrap()
}

#[tokio::test]
async fn the_cutover_migration_keeps_balances_and_renames_the_old_payment_tables() {
    let name = format!("yalr_test_{}_cutover", std::process::id());
    let mut admin = MySqlConnection::connect(&server()).await.unwrap();
    admin.execute(format!("DROP DATABASE IF EXISTS `{name}`").as_str()).await.unwrap();
    admin.execute(format!("CREATE DATABASE `{name}`").as_str()).await.unwrap();
    let mut conn = MySqlConnection::connect(&format!("{}/{name}", server().trim_end_matches('/'))).await.unwrap();

    migrate(&mut conn, true).await;
    for statement in [
        "INSERT INTO users (id, username, user_type, is_admin) VALUES (1, 'alice', 0, 0), (2, 'anon_abcdef123456', 0, 0)",
        "INSERT INTO user_balances (user_id, balance_msat, lifetime_deposited_msat) VALUES (1, 5000000, 9000000), (2, 100000, 100000)",
        "INSERT INTO user_currency_balances (user_id, currency, balance_milli, lifetime_deposited_milli) VALUES (1, 'eur', 7000000, 9000000)",
        "INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id) VALUES (1, 9000000, 'deposit', 'ph_paid')",
        "INSERT INTO lightning_invoices (user_id, payment_hash, bolt11, amount_msat, amount_sats, status, purpose, public_id) VALUES \
         (1, 'ph_paid', 'lnbc1', 9000000, 9000, 'paid', 'topup', 'pub1'), \
         (2, 'ph_open', 'lnbc2', 100000, 100, 'pending', 'create', 'pub2')",
        "INSERT INTO fiat_orders (public_id, user_id, external_id, currency, amount_milli, status) VALUES \
         ('o1', 1, 'rev_done', 'eur', 9000000, 'completed'), ('o2', 1, 'rev_open', 'eur', 1000000, 'pending')",
        "INSERT INTO user_payment_methods (id, user_id, external_customer_id, external_id) VALUES (1, 1, 'sealed', 'pm_1')",
        "INSERT INTO auto_reload_settings (user_id, currency, enabled, threshold_milli, amount_milli, monthly_cap_milli, payment_method_id) VALUES (1, 'eur', 1, 500000, 2000000, 9000000, 1)",
        "INSERT INTO payouts (user_id, currency, amount, method, destination, status) VALUES (1, 'sats', 1000, 'lightning', 'x', 'completed')",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }

    migrate(&mut conn, false).await;

    assert_eq!(count(&mut conn, "SELECT balance_msat FROM user_balances WHERE user_id = 1").await, 5_000_000);
    assert_eq!(count(&mut conn, "SELECT balance_milli FROM user_currency_balances WHERE user_id = 1 AND currency = 'eur'").await, 7_000_000);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM balance_transactions").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM payouts").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_lightning_invoices WHERE status = 'paid'").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_lightning_invoices WHERE status = 'expired' AND payment_hash = 'ph_open'").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_fiat_orders WHERE status = 'failed' AND external_id = 'rev_open'").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_fiat_orders WHERE status = 'completed'").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_user_payment_methods").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM legacy_auto_reload_settings").await, 1);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM auto_reload_settings").await, 0);
    assert_eq!(count(&mut conn, "SELECT COUNT(*) FROM gateway_payments").await, 0);
    for old in ["lightning_invoices", "fiat_orders", "user_payment_methods"] {
        let exists = count(&mut conn, &format!("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = '{old}'")).await;
        assert_eq!(exists, 0, "{old} should have been renamed");
    }

    conn.close().await.unwrap();
    admin.execute(format!("DROP DATABASE `{name}`").as_str()).await.unwrap();
}
