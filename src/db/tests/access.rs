use crate::db::*;
use super::setup_test_db;

#[tokio::test]
async fn balances_for_users_returns_only_funded_users() {
    let db = Database::test().await;
    for name in ["a", "b"] {
        sqlx::query("INSERT INTO users (username, user_type, is_admin) VALUES (?, 0, 0)")
            .bind(name)
            .execute(&db.pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO user_balances (user_id, balance_msat) VALUES (2, 5000)")
        .execute(&db.pool)
        .await
        .unwrap();
    let balances = db.balances_for_users(&[1, 2]).await.unwrap();
    assert_eq!(balances.get(&2), Some(&5000));
    assert_eq!(balances.get(&1), None);
    assert!(db.balances_for_users(&[]).await.unwrap().is_empty());
}

// ── Model Access Control Tests ─────────────────────────────────────

#[tokio::test]
async fn test_check_model_access_no_rules_allows() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    // No permission rules exist, should allow by default
    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);
}

#[tokio::test]
async fn test_check_model_access_rules_exist_but_none_match_denies() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    // User has rules for "code" but tries to use "gpt-4" — should deny
    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "code",
        allow: true,
    }).await.unwrap();

    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Deny);

    // But "code" itself should still be allowed
    let result2 = db.check_model_access(user.id, "code").await.unwrap();
    assert_eq!(result2, ModelAccess::Allow);
}

async fn priced_user(db: &Database, name: &str, admin: bool) -> User {
    db.create_user(NewUser {
        username: Some(name),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: admin,
    }).await.unwrap()
}

async fn price(db: &Database, model: &str, advertised: bool) {
    sqlx::query("INSERT INTO model_pricing (model_name, is_advertised) VALUES (?, ?)")
        .bind(model)
        .bind(advertised)
        .execute(&db.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn cache_prices_round_trip_and_update_independently() {
    let db = setup_test_db().await;
    let created = db
        .create_model_pricing(NewModelPricing {
            model_name: "m",
            is_advertised: true,
            is_free: false,
            price_per_1m_input_sats: Some(1000),
            price_per_1m_output_sats: Some(4000),
            price_per_1m_cache_read_sats: Some(100),
            price_per_1m_cache_write_sats: None,
            price_per_request_sats: Some(1),
            context_window: None,
            max_output_tokens: None,
        })
        .await
        .unwrap();
    assert_eq!((created.price_per_1m_cache_read_sats, created.price_per_1m_cache_write_sats), (Some(100), None));

    let updated = db
        .update_model_pricing(
            "m",
            UpdateModelPricing {
                is_advertised: None,
                is_free: None,
                price_per_1m_input_sats: None,
                price_per_1m_output_sats: None,
                price_per_1m_cache_read_sats: None,
                price_per_1m_cache_write_sats: Some(Some(1250)),
                price_per_request_sats: None,
                context_window: None,
                max_output_tokens: None,
            },
        )
        .await
        .unwrap();
    assert_eq!((updated.price_per_1m_cache_read_sats, updated.price_per_1m_cache_write_sats), (Some(100), Some(1250)));

    let cleared = db
        .update_model_pricing(
            "m",
            UpdateModelPricing {
                is_advertised: None,
                is_free: None,
                price_per_1m_input_sats: None,
                price_per_1m_output_sats: None,
                price_per_1m_cache_read_sats: Some(None),
                price_per_1m_cache_write_sats: None,
                price_per_request_sats: None,
                context_window: None,
                max_output_tokens: None,
            },
        )
        .await
        .unwrap();
    assert_eq!((cleared.price_per_1m_cache_read_sats, cleared.price_per_1m_cache_write_sats), (None, Some(1250)));
}

#[tokio::test]
async fn users_page_newest_first_and_search() {
    let db = setup_test_db().await;
    assert!(!db.has_users().await.unwrap());
    for name in ["alice", "bob", "anon_x1", "anon_x2", "carol"] {
        db.create_user(NewUser {
            username: Some(name),
            password_hash: None,
            external_id: None,
            user_type: UserType::Internal,
            is_admin: false,
        })
        .await
        .unwrap();
    }
    assert!(db.has_users().await.unwrap());
    let page = |limit, before, search| Page { limit, before, search };
    let first = db.list_users(page(2, None, None)).await.unwrap();
    let names: Vec<_> = first.iter().map(|u| u.username.clone().unwrap()).collect();
    assert_eq!(names, vec!["carol", "anon_x2"]);
    let next = db.list_users(page(2, Some(first[1].id), None)).await.unwrap();
    let names: Vec<_> = next.iter().map(|u| u.username.clone().unwrap()).collect();
    assert_eq!(names, vec!["anon_x1", "bob"]);
    let found = db.list_users(page(10, None, Some("anon_"))).await.unwrap();
    assert_eq!(found.len(), 2);
}

#[tokio::test]
async fn plain_users_get_only_models_for_sale() {
    let db = setup_test_db().await;
    let user = priced_user(&db, "buyer", false).await;
    price(&db, "sale", true).await;
    price(&db, "hidden", false).await;

    let access = |m: &'static str| {
        let db = db.clone();
        async move { db.effective_model_access(user.id, m, true).await.unwrap() }
    };
    assert_eq!(access("sale").await, ModelAccess::Allow);
    assert_eq!(access("hidden").await, ModelAccess::Deny);
    assert_eq!(access("unpriced").await, ModelAccess::Deny);
}

#[tokio::test]
async fn explicit_rules_override_the_sale_default() {
    let db = setup_test_db().await;
    let user = priced_user(&db, "buyer", false).await;
    price(&db, "sale", true).await;
    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "unpriced",
        allow: true,
    }).await.unwrap();

    assert_eq!(db.effective_model_access(user.id, "unpriced", true).await.unwrap(), ModelAccess::Allow);
    assert_eq!(db.effective_model_access(user.id, "sale", true).await.unwrap(), ModelAccess::Deny);
}

#[tokio::test]
async fn admins_and_exempt_users_keep_open_access() {
    let db = setup_test_db().await;
    let admin = priced_user(&db, "root", true).await;
    let friend = priced_user(&db, "friend", false).await;
    sqlx::query("INSERT INTO billing_exempt_users (user_id) VALUES (?)")
        .bind(friend.id)
        .execute(&db.pool)
        .await
        .unwrap();

    for id in [admin.id, friend.id] {
        assert_eq!(db.effective_model_access(id, "unpriced", true).await.unwrap(), ModelAccess::Allow);
    }
}

#[tokio::test]
async fn sale_default_is_off_without_payments() {
    let db = setup_test_db().await;
    let user = priced_user(&db, "buyer", false).await;
    assert_eq!(db.effective_model_access(user.id, "unpriced", false).await.unwrap(), ModelAccess::Allow);
}

#[tokio::test]
async fn test_check_model_access_explicit_allow() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: true,
    }).await.unwrap();

    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);
}

#[tokio::test]
async fn test_check_model_access_explicit_deny() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: false,
    }).await.unwrap();

    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Deny);
}

#[tokio::test]
async fn test_check_model_access_wildcard_allow() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "*",
        allow: true,
    }).await.unwrap();

    // Wildcard should apply to all models
    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);

    let result2 = db.check_model_access(user.id, "claude-3").await.unwrap();
    assert_eq!(result2, ModelAccess::Allow);
}

#[tokio::test]
async fn test_check_model_access_wildcard_deny() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "*",
        allow: false,
    }).await.unwrap();

    // Wildcard deny should apply to all models
    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Deny);
}

#[tokio::test]
async fn test_check_model_access_exact_overrides_wildcard() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    // Set wildcard deny
    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "*",
        allow: false,
    }).await.unwrap();

    // Set specific allow for gpt-4
    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: true,
    }).await.unwrap();

    // Exact match should take precedence
    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);

    // Other models should still be denied
    let result2 = db.check_model_access(user.id, "claude-3").await.unwrap();
    assert_eq!(result2, ModelAccess::Deny);
}

#[tokio::test]
async fn test_list_user_model_permissions() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: true,
    }).await.unwrap();
    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "claude-3",
        allow: false,
    }).await.unwrap();

    let permissions = db.list_user_model_permissions(user.id).await.unwrap();
    assert_eq!(permissions.len(), 2);
    assert!(permissions.iter().any(|p| p.model == "gpt-4" && p.allow));
    assert!(permissions.iter().any(|p| p.model == "claude-3" && !p.allow));
}

#[tokio::test]
async fn test_update_user_model_permission() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: false,
    }).await.unwrap();

    db.update_user_model_permission(user.id, "gpt-4", UpdateUserModelPermission {
        allow: Some(true),
    }).await.unwrap();

    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);
}

#[tokio::test]
async fn test_delete_user_model_permission() {
    let db = setup_test_db().await;
    let user = db.create_user(NewUser {
        username: Some("test_user"),
        password_hash: None,
        external_id: None,
        user_type: UserType::Internal,
        is_admin: false,
    }).await.unwrap();

    db.create_user_model_permission(NewUserModelPermission {
        user_id: user.id,
        model: "gpt-4",
        allow: false,
    }).await.unwrap();

    let deleted = db.delete_user_model_permission(user.id, "gpt-4").await.unwrap();
    assert!(deleted);

    // Should return Allow after deletion (no rule = allow by default)
    let result = db.check_model_access(user.id, "gpt-4").await.unwrap();
    assert_eq!(result, ModelAccess::Allow);
}
