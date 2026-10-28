use crate::db::*;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

async fn legacy_sqlite() -> (std::path::PathBuf, sqlx::SqlitePool) {
    let dir = std::env::temp_dir().join(format!("yalr-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("legacy.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=rwc", file.display())).unwrap();
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    sqlx::migrate!("./migrations-sqlite").run(&pool).await.unwrap();
    let statements = [
        "INSERT INTO providers (name, slug, base_url, api_key, provider_type, oauth_expires_at) VALUES ('Local', 'local', 'http://x', 'k', 1, 1792504800000)",
        "INSERT INTO routing_config (name, strategy, input_modalities) VALUES ('code-high', 'round_robin', '[\"text\",\"image\"]')",
        "INSERT INTO routing_config_providers (routing_config_id, provider_id, model, weight) VALUES (1, 1, 'qwen', 50)",
        "INSERT INTO users (username, user_type, is_admin, created_at) VALUES ('Alice', 0, 1, '2026-10-20 14:00:00')",
        "INSERT INTO users (username, external_id, user_type, is_admin) VALUES ('nostr_ab', 'abcdef', 1, 0)",
        "INSERT INTO api_keys (key_hash, name, user_id, last_four, expires_at) VALUES ('hash', 'main', 1, 'abcd', '2030-01-01 00:00:00')",
        "INSERT INTO user_balances (user_id, balance_msat, lifetime_deposited_msat) VALUES (1, 5000, 9000)",
        "INSERT INTO balance_transactions (user_id, amount_msat, transaction_type, reference_id, metadata) VALUES (1, 9000, 'deposit', 'inv', '{\"model\":\"m\"}')",
        "INSERT INTO lightning_invoices (user_id, payment_hash, bolt11, amount_msat, amount_sats, status, expires_at, paid_at, purpose, public_id) VALUES (1, 'ph', 'lnbc1', 9000, 9, 'paid', '2026-10-20 15:00:00', '2026-10-20 14:05:00', 'create', 'pub')",
        "INSERT INTO model_pricing (model_name, is_advertised, price_per_1m_input_sats, context_window) VALUES ('code-high', 1, 21, 256000)",
        "INSERT INTO api_key_usage (api_key_id, hour, model, requests, cost_msat) VALUES (1, '2026-10-20T14:00:00Z', 'code-high', 3, 70)",
        "INSERT INTO metrics_history (timestamp_ms, snapshot_json) VALUES (1792504800000, '{}')",
        "INSERT INTO server_secret (id, secret) VALUES (1, 'secret')",
        "INSERT INTO request_log (request_id, started_at_ms, endpoint, stream, model, attempts, status) VALUES ('r1', 1, '/v1/chat/completions', 1, 'code-high', 1, 'ok')",
        "INSERT INTO request_log_body (log_id, client_request_packed) VALUES (1, X'010203')",
    ];
    for statement in statements {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    (file, pool)
}

#[tokio::test]
async fn a_legacy_sqlite_database_imports_with_ids_types_and_timestamps_intact() {
    let (file, legacy) = legacy_sqlite().await;
    legacy.close().await;
    let db = Database::test().await;
    let report = db.import_sqlite(&format!("sqlite://{}", file.display())).await.unwrap();
    let count = |table: &str| report.tables.iter().find(|(t, _)| t == table).unwrap().1;
    assert_eq!((count("users"), count("request_log_body"), count("api_key_usage")), (2, 1, 1));

    let alice = db.get_user_by_username("Alice").await.unwrap().unwrap();
    assert!(alice.is_admin);
    assert_eq!(alice.created_at.to_rfc3339(), "2026-10-20T14:00:00+00:00");
    assert!(db.get_user_by_username("alice").await.unwrap().is_none());
    let nostr = db.get_user_by_external_id("abcdef", UserType::Nostr).await.unwrap().unwrap();
    assert_eq!(nostr.id, 2);

    let (key, user) = db.authenticate_api_key("hash").await.unwrap().unwrap();
    assert_eq!((key.id, user.id), (1, 1));
    assert_eq!(db.get_user_balance(1).await.unwrap().unwrap().lifetime_deposited_msat, 9000);

    let invoice = db.get_lightning_invoice_by_hash("ph").await.unwrap().unwrap();
    assert_eq!(invoice.paid_at.unwrap().to_rfc3339(), "2026-10-20T14:05:00+00:00");
    assert_eq!(db.get_public_invoice("pub").await.unwrap().unwrap().purpose, "create");

    let provider = db.get_provider_by_slug("local").await.unwrap().unwrap();
    assert_eq!((provider.provider_type, provider.oauth_expires_at), (ProviderType::LlamaCpp, Some(1792504800000)));
    let rc = db.get_routing_config_by_name("code-high").await.unwrap().unwrap();
    assert_eq!(rc.input_modalities.as_deref(), Some("[\"text\",\"image\"]"));
    assert_eq!(db.list_active_routing_config_providers(rc.id).await.unwrap()[0].model.as_deref(), Some("qwen"));
    assert_eq!(db.get_model_pricing("code-high").await.unwrap().unwrap().context_window, Some(256000));
    assert_eq!(db.server_secret().await.unwrap(), "secret");
    assert_eq!(db.request_log_bodies(1).await.unwrap().unwrap().client_request_packed, Some(vec![1, 2, 3]));
    assert_eq!(db.api_key_usage_for_user(1).await.unwrap()[&1].cost_msat, 70);

    let fresh = db.create_user(NewUser { username: Some("bob"), password_hash: None, external_id: None, user_type: UserType::Internal, is_admin: false }).await.unwrap();
    assert_eq!(fresh.id, 3);

    let again = db.import_sqlite(&format!("sqlite://{}", file.display())).await;
    assert!(again.is_err());
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[tokio::test]
async fn batches_larger_than_the_server_packet_limit_are_split() {
    let (file, legacy) = legacy_sqlite().await;
    let snapshot = format!("{{\"pad\":\"{}\"}}", "x".repeat(200_000));
    for t in 0..200i64 {
        sqlx::query("INSERT INTO metrics_history (timestamp_ms, snapshot_json) VALUES (?, ?)")
            .bind(t)
            .bind(&snapshot)
            .execute(&legacy)
            .await
            .unwrap();
    }
    legacy.close().await;
    let db = Database::test().await;
    let report = db.import_sqlite(&format!("sqlite://{}", file.display())).await.unwrap();
    let copied = report.tables.iter().find(|(t, _)| t == "metrics_history").unwrap().1;
    assert_eq!(copied, 201);
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[tokio::test]
async fn a_single_body_over_the_packet_limit_is_skipped_and_reported() {
    let (file, legacy) = legacy_sqlite().await;
    let db = Database::test().await;
    let limit = db.max_packet_bytes().await.unwrap() as usize;
    sqlx::query("INSERT INTO request_log (request_id, started_at_ms, endpoint, stream, model, attempts, status) VALUES ('big', 2, '/v1/chat/completions', 1, 'm', 1, 'ok')")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::query("INSERT INTO request_log_body (log_id, client_request_packed) VALUES (2, ?)")
        .bind(vec![7u8; limit + 1])
        .execute(&legacy)
        .await
        .unwrap();
    legacy.close().await;
    let report = db.import_sqlite(&format!("sqlite://{}", file.display())).await.unwrap();
    assert_eq!(report.tables.iter().find(|(t, _)| t == "request_log_body").unwrap().1, 1);
    assert_eq!(report.skipped, vec![("request_log_body".to_string(), 1)]);
    assert!(db.request_log_detail(2).await.unwrap().is_some());
    assert!(db.request_log_bodies(2).await.unwrap().is_none());
    std::fs::remove_dir_all(file.parent().unwrap()).ok();
}
