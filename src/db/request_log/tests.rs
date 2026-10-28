use super::*;

fn record(request_id: &str, started_at_ms: i64, with_bodies: bool) -> Record {
    Record {
        request_id: request_id.to_string(),
        started_at_ms,
        endpoint: "/v1/chat/completions".into(),
        stream: true,
        model: "code-high".into(),
        status: "ok".into(),
        attempts: 1,
        total_ms: 1200,
        prompt_tokens: Some(1000),
        cached_tokens: Some(900),
        client_request_packed: with_bodies.then(|| vec![1, 2, 3]),
        upstream_request_packed: with_bodies.then(|| vec![4, 5]),
        response_packed: with_bodies.then(|| vec![6]),
        ..Default::default()
    }
}

fn sized(request_id: &str, started_at_ms: i64, bytes: usize) -> Record {
    Record {
        client_request_packed: Some(vec![7; bytes]),
        upstream_request_packed: Some(vec![8; bytes]),
        response_packed: None,
        ..record(request_id, started_at_ms, false)
    }
}

#[tokio::test]
async fn insert_batch_writes_every_record_in_one_transaction() {
    let db = Database::test().await;
    let records: Vec<Record> = (0..5).map(|n| record(&format!("b{n}"), 1_000 + n, n % 2 == 0)).collect();
    let ids = db.insert_request_logs(&records).await.unwrap();
    assert_eq!(ids.len(), 5);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_log").fetch_one(&db.pool).await.unwrap();
    let bodies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM request_log_body").fetch_one(&db.pool).await.unwrap();
    assert_eq!((rows, bodies), (5, 3));
}

#[tokio::test]
async fn insert_then_list_detail_and_bodies_round_trip() {
    let db = Database::test().await;
    let id = db.insert_request_log(record("r1", 1_000, true)).await.unwrap();
    let rows = db.list_request_logs(10, Filter::default()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id, "r1");
    assert_eq!(rows[0].cached_tokens, Some(900));
    let found = db.request_log_detail(id).await.unwrap().unwrap();
    assert_eq!(found.model, "code-high");
    assert!(found.stream);
    let stored = db.request_log_bodies(id).await.unwrap().unwrap();
    assert_eq!(stored.client_request_packed, Some(vec![1, 2, 3]));
    assert_eq!(stored.response_packed, Some(vec![6]));
}

#[tokio::test]
async fn list_scopes_to_user_and_key() {
    let db = Database::test().await;
    for (id, user, key) in [("a", 1, 10), ("b", 1, 11), ("c", 2, 12)] {
        let mut r = record(id, 1, false);
        r.user_id = Some(user);
        r.api_key_id = Some(key);
        db.insert_request_log(r).await.unwrap();
    }
    let mine = db.list_request_logs(10, Filter { user_id: Some(1), ..Default::default() }).await.unwrap();
    assert_eq!(mine.len(), 2);
    let one = db.list_request_logs(10, Filter { api_key_id: Some(11), ..Default::default() }).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].request_id, "b");
    assert_eq!(one[0].api_key_id, Some(11));
    let late = db.list_request_logs(10, Filter { since_ms: Some(2), ..Default::default() }).await.unwrap();
    assert!(late.is_empty());
}

#[tokio::test]
async fn request_id_resolves_to_row_id() {
    let db = Database::test().await;
    let id = db.insert_request_log(record("r1", 1_000, false)).await.unwrap();
    assert_eq!(db.request_log_id("r1").await.unwrap(), Some(id));
    assert_eq!(db.request_log_id("missing").await.unwrap(), None);
}

#[tokio::test]
async fn rows_without_bodies_have_no_body_row() {
    let db = Database::test().await;
    let id = db.insert_request_log(record("r1", 1_000, false)).await.unwrap();
    assert!(db.request_log_bodies(id).await.unwrap().is_none());
    assert!(!db.request_log_has_bodies(id).await.unwrap());
    let with = db.insert_request_log(record("r2", 1_000, true)).await.unwrap();
    assert!(db.request_log_has_bodies(with).await.unwrap());
}

#[tokio::test]
async fn list_filters_and_pages_newest_first() {
    let db = Database::test().await;
    let first = db.insert_request_log(record("a", 1, false)).await.unwrap();
    let mut failed = record("b", 2, false);
    failed.status = "error".into();
    let second = db.insert_request_log(failed).await.unwrap();
    let all = db.list_request_logs(10, Filter::default()).await.unwrap();
    assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), vec![second, first]);
    let errors = db.list_request_logs(10, Filter { status: Some("error"), ..Default::default() }).await.unwrap();
    assert_eq!(errors.len(), 1);
    let older = db.list_request_logs(10, Filter { before: Some(second), ..Default::default() }).await.unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].id, first);
}

#[tokio::test]
async fn prune_drops_old_bodies_but_keeps_metadata() {
    let db = Database::test().await;
    let now = 100 * DAY_MS;
    let old = db.insert_request_log(record("old", now - 8 * DAY_MS, true)).await.unwrap();
    let fresh = db.insert_request_log(record("fresh", now - DAY_MS, true)).await.unwrap();
    db.prune_request_logs(now, &Retention { body_days: 7, metadata_days: 0, max_body_bytes: 0 }).await.unwrap();
    assert!(db.request_log_bodies(old).await.unwrap().is_none());
    assert!(db.request_log_bodies(fresh).await.unwrap().is_some());
    assert!(db.request_log_detail(old).await.unwrap().is_some());
}

#[tokio::test]
async fn prune_can_also_expire_metadata() {
    let db = Database::test().await;
    let now = 100 * DAY_MS;
    let old = db.insert_request_log(record("old", now - 40 * DAY_MS, true)).await.unwrap();
    db.prune_request_logs(now, &Retention { body_days: 7, metadata_days: 30, max_body_bytes: 0 }).await.unwrap();
    assert!(db.request_log_detail(old).await.unwrap().is_none());
    assert!(db.request_log_bodies(old).await.unwrap().is_none());
}

#[tokio::test]
async fn prune_keeps_the_newest_bodies_within_the_byte_cap() {
    let db = Database::test().await;
    let now = 100 * DAY_MS;
    let mut ids = Vec::new();
    for n in 0..6 {
        ids.push(db.insert_request_log(sized(&format!("r{n}"), now - (6 - n) * 1000, 1000)).await.unwrap());
    }
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 12_000);
    let keep = Retention { body_days: 7, metadata_days: 0, max_body_bytes: 5_000 };
    db.prune_request_logs(now, &keep).await.unwrap();
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 4_000);
    for id in &ids[..4] {
        assert!(db.request_log_bodies(*id).await.unwrap().is_none());
        assert!(db.request_log_detail(*id).await.unwrap().is_some());
    }
    for id in &ids[4..] {
        assert!(db.request_log_bodies(*id).await.unwrap().is_some());
    }
}

#[tokio::test]
async fn a_zero_byte_cap_means_unlimited() {
    let db = Database::test().await;
    let now = 100 * DAY_MS;
    for n in 0..3 {
        db.insert_request_log(sized(&format!("r{n}"), now - 1000, 1000)).await.unwrap();
    }
    db.prune_request_logs(now, &Retention { body_days: 7, metadata_days: 0, max_body_bytes: 0 }).await.unwrap();
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 6_000);
}

#[tokio::test]
async fn filtered_lists_use_an_index_without_sorting() {
    use sqlx::Row;
    let db = Database::test().await;
    for n in 0..50 {
        let mut r = record(&format!("r{n}"), n, false);
        r.user_id = Some(n % 5);
        r.api_key_id = Some(n % 7);
        db.insert_request_log(r).await.unwrap();
    }
    sqlx::query("ANALYZE TABLE request_log").execute(&db.pool).await.unwrap();
    for filter in ["model = 'm'", "status = 'error'", "user_id = 1", "api_key_id = 1", "request_id = 'x'"] {
        let sql = format!("EXPLAIN SELECT id FROM request_log WHERE id < 1000 AND {filter} ORDER BY id DESC LIMIT 100");
        let row = sqlx::query(&sql).fetch_one(&db.pool).await.unwrap();
        let key: Option<String> = row.try_get("key").unwrap();
        let extra: Option<String> = row.try_get("Extra").unwrap();
        assert!(key.is_some(), "{filter}: no index");
        assert!(!extra.unwrap_or_default().contains("filesort"), "{filter}: sorts");
    }
}

#[tokio::test]
async fn a_body_over_the_packet_limit_is_dropped_without_losing_the_batch() {
    let db = Database::test().await;
    let limit = db.max_packet_bytes().await.unwrap();
    let huge = Record { client_request_packed: Some(vec![9; limit as usize + 1]), ..record("huge", 2, false) };
    let ids = db.insert_request_logs(&[record("small", 1, true), huge]).await.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(db.request_log_bodies(ids[0]).await.unwrap().is_some());
    assert!(db.request_log_detail(ids[1]).await.unwrap().is_some());
    assert!(db.request_log_bodies(ids[1]).await.unwrap().is_none());
}

#[tokio::test]
async fn body_size_queries_read_the_size_index_not_the_blobs() {
    use sqlx::Row;
    let db = Database::test().await;
    for n in 0..20 {
        db.insert_request_log(sized(&format!("r{n}"), n, 1000)).await.unwrap();
    }
    sqlx::query("ANALYZE TABLE request_log_body").execute(&db.pool).await.unwrap();
    for sql in [BODY_BYTES_TOTAL, BODIES_OVER_BYTE_CAP] {
        let rows = sqlx::query(&format!("EXPLAIN {sql}")).bind(1i64).fetch_all(&db.pool).await.unwrap();
        let body_scan = rows
            .iter()
            .find(|r| r.try_get::<Option<String>, _>("table").unwrap().as_deref() == Some("request_log_body"))
            .expect("plan touches request_log_body");
        let extra: Option<String> = body_scan.try_get("Extra").unwrap();
        assert!(extra.unwrap_or_default().contains("Using index"), "{sql} reads the blobs");
    }
}

#[tokio::test]
async fn bodies_without_a_recorded_size_are_backfilled_in_batches() {
    let db = Database::test().await;
    for n in 0..5 {
        db.insert_request_log(sized(&format!("r{n}"), n, 1000)).await.unwrap();
    }
    db.execute_raw("UPDATE request_log_body SET size_bytes = NULL").await.unwrap();
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 0);
    assert_eq!(db.backfill_request_log_body_sizes(2).await.unwrap(), 2);
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 4_000);
    while db.backfill_request_log_body_sizes(2).await.unwrap() > 0 {}
    assert_eq!(db.request_log_body_bytes().await.unwrap(), 10_000);
}
