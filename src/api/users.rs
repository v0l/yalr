use crate::db::UserType;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize)]
pub struct UserResponse {
    pub id: i64,
    pub username: Option<String>,
    pub external_id: Option<String>,
    pub user_type: String,
    pub is_admin: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct UserDetailResponse {
    pub user: UserResponse,
    pub api_keys: Vec<UserApiKeyResponse>,
}

#[derive(Serialize)]
pub struct UserApiKeyResponse {
    pub id: i64,
    pub name: String,
    pub last_four: String,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub external_id: Option<String>,
    pub user_type: String,
    pub is_admin: bool,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    pub is_admin: Option<bool>,
}

#[derive(Serialize)]
pub struct UserCreateResponse {
    pub message: String,
    pub user: UserResponse,
}

#[derive(Serialize)]
pub struct UserDeleteResponse {
    pub message: String,
}

#[axum::debug_handler]
pub async fn list_users(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<UserResponse>> {
    let users = match state.db.list_users().await {
        Ok(users) => users,
        Err(e) => {
            tracing::error!("Failed to list users from DB: {}", e);
            return Json(vec![]);
        }
    };

    let users_list: Vec<UserResponse> = users
        .into_iter()
        .map(|u| UserResponse {
            id: u.id,
            username: u.username,
            external_id: u.external_id,
            user_type: match u.user_type {
                UserType::Internal => "internal".to_string(),
                UserType::Nostr => "nostr".to_string(),
                UserType::OAuth => "oauth".to_string(),
            },
            is_admin: u.is_admin,
            created_at: u.created_at,
            updated_at: u.updated_at,
        })
        .collect();

    Json(users_list)
}

#[axum::debug_handler]
pub async fn create_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<UserCreateResponse>, (axum::http::StatusCode, String)> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use rand::rngs::OsRng;

    if request.username.is_none() && request.external_id.is_none() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "Either username or external_id is required".to_string()));
    }

    if let Some(username) = &request.username {
        if state.db.get_user_by_username(username).await.unwrap_or(None).is_some() {
            return Err((axum::http::StatusCode::BAD_REQUEST, format!("User '{}' already exists", username)));
        }
    }

    let user_type = match request.user_type.as_str() {
        "internal" => UserType::Internal,
        "nostr" => UserType::Nostr,
        "oauth" => UserType::OAuth,
        _ => UserType::Internal,
    };

    let password_hash = if let Some(password) = &request.password {
        if user_type == UserType::Internal {
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            Some(
                argon2
                    .hash_password(password.as_bytes(), &salt)
                    .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                    .to_string()
            )
        } else {
            None
        }
    } else {
        None
    };

    // Empty string external_id should be treated as None to avoid UNIQUE constraint
    // violations on (external_id, user_type) - SQLite treats NULLs as distinct.
    let external_id = request.external_id
        .as_deref()
        .filter(|s| !s.is_empty());

    let new_user = crate::db::NewUser {
        username: request.username.as_deref(),
        password_hash: password_hash.as_deref(),
        external_id,
        user_type,
        is_admin: request.is_admin,
    };

    let user = state.db.create_user(new_user)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: user.id,
        username: user.username,
        external_id: user.external_id,
        user_type: match user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: user.is_admin,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    Ok(Json(UserCreateResponse {
        message: "User created successfully".to_string(),
        user: user_response,
    }))
}

#[axum::debug_handler]
pub async fn update_user(
    Path(id): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, (axum::http::StatusCode, String)> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use rand::rngs::OsRng;

    let user_id: i64 = id.parse()
        .map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "Invalid user ID".to_string()))?;

    let existing_user = state.db.get_user_by_id(user_id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "User not found".to_string()))?;

    if let Some(new_username) = &request.username {
        if Some(new_username.as_str()) != existing_user.username.as_deref()
            && state.db.get_user_by_username(new_username).await.unwrap_or(None).is_some() {
                return Err((axum::http::StatusCode::BAD_REQUEST, format!("User '{}' already exists", new_username)));
            }
    }

    let mut updates = Vec::new();
    let mut bindings: Vec<String> = Vec::new();

    if let Some(new_username) = &request.username {
        updates.push("username = ?".to_string());
        bindings.push(new_username.clone());
    }

    if let Some(new_password) = &request.password {
        if existing_user.user_type == UserType::Internal {
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let hash = argon2
                .hash_password(new_password.as_bytes(), &salt)
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .to_string();
            updates.push("password_hash = ?".to_string());
            bindings.push(hash);
        }
    }

    if request.is_admin.is_some() {
        updates.push("is_admin = ?".to_string());
    }

    if updates.is_empty() {
        return Err((axum::http::StatusCode::BAD_REQUEST, "No updates provided".to_string()));
    }

    let mut query = format!("UPDATE users SET updated_at = CURRENT_TIMESTAMP, {}", updates.join(", "));
    query.push_str(" WHERE id = ? RETURNING *");

    let mut query_builder = sqlx::query_as::<_, crate::db::User>(&query);

    for binding in &bindings {
        query_builder = query_builder.bind(binding);
    }

    if let Some(is_admin) = request.is_admin {
        query_builder = query_builder.bind(is_admin);
    }

    query_builder = query_builder.bind(user_id);

    let updated_user = query_builder
        .fetch_one(&state.db.pool)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: updated_user.id,
        username: updated_user.username,
        external_id: updated_user.external_id,
        user_type: match updated_user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: updated_user.is_admin,
        created_at: updated_user.created_at,
        updated_at: updated_user.updated_at,
    };

    Ok(Json(user_response))
}

#[axum::debug_handler]
pub async fn delete_user(
    Path(id): Path<String>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<UserDeleteResponse>, (axum::http::StatusCode, String)> {
    let user_id: i64 = id.parse()
        .map_err(|_| (axum::http::StatusCode::BAD_REQUEST, "Invalid user ID".to_string()))?;

    state.db.delete_user(user_id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(UserDeleteResponse {
        message: "User deleted successfully".to_string(),
    }))
}

#[axum::debug_handler]
pub async fn get_user(
    Path(id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<UserDetailResponse>, (axum::http::StatusCode, String)> {
    let user = state.db.get_user_by_id(id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let api_keys = state.db.list_api_keys_for_user(id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user_response = UserResponse {
        id: user.id,
        username: user.username,
        external_id: user.external_id,
        user_type: match user.user_type {
            UserType::Internal => "internal".to_string(),
            UserType::Nostr => "nostr".to_string(),
            UserType::OAuth => "oauth".to_string(),
        },
        is_admin: user.is_admin,
        created_at: user.created_at,
        updated_at: user.updated_at,
    };

    let api_keys_response: Vec<UserApiKeyResponse> = api_keys.into_iter().map(|k| {
        UserApiKeyResponse {
            id: k.id,
            name: k.name,
            last_four: k.last_four,
            created_at: k.created_at,
            expires_at: k.expires_at.map(|e| e.to_string()),
            is_active: k.is_active,
        }
    }).collect();

    Ok(Json(UserDetailResponse {
        user: user_response,
        api_keys: api_keys_response,
    }))
}

// ============================================================================
// Admin Payment Management Handlers
// ============================================================================

/// GET /api/payments/balances — List all user balances (admin)
#[axum::debug_handler]
pub async fn list_all_balances(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<crate::db::UserBalanceWithUsername>> {
    let balances = state.db.list_all_user_balances().await.unwrap_or_default();
    Json(balances)
}

/// GET /api/payments/balances/:user_id — Get a single user's balance + recent transactions
#[axum::debug_handler]
pub async fn get_user_balance_details(
    Path(user_id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let balance = state.db.get_user_balance(user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let transactions = state.db.get_user_transactions(user_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let balance_msat = balance.as_ref().map(|b| b.balance_msat).unwrap_or(0);
    let lifetime_deposited = balance.as_ref().map(|b| b.lifetime_deposited_msat).unwrap_or(0);

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "balance_msat": balance_msat,
        "lifetime_deposited_msat": lifetime_deposited,
        "transactions": transactions,
    })))
}

/// POST /api/payments/credit — Admin manually credits a user's balance
#[derive(Deserialize)]
pub struct AdminCreditRequest {
    pub user_id: i64,
    pub amount_sats: u64,
    pub reason: Option<String>,
}

#[axum::debug_handler]
pub async fn admin_credit_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<AdminCreditRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount_sats == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }

    let payments = state.payments_state.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))?;

    let amount_msat = (req.amount_sats as i64) * 1000;
    let ref_id = format!("admin-credit-{}", chrono::Utc::now().timestamp_millis());
    let reason = req.reason.as_deref().unwrap_or("admin_credit");

    let new_balance = payments.balance_service
        .credit(req.user_id, amount_msat, reason, &ref_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        admin_action = "credit",
        user_id = req.user_id,
        amount_msat = amount_msat,
        new_balance_msat = new_balance,
        reason = reason,
        "Admin manual credit"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "user_id": req.user_id,
        "credited_sats": req.amount_sats,
        "new_balance_msat": new_balance,
        "new_balance_sats": new_balance / 1000,
        "reason": reason,
    })))
}

/// POST /api/payments/debit — Admin manually debits a user's balance
#[derive(Deserialize)]
pub struct AdminDebitRequest {
    pub user_id: i64,
    pub amount_sats: u64,
    pub reason: Option<String>,
}

#[axum::debug_handler]
pub async fn admin_debit_user(
    State(state): State<std::sync::Arc<AppState>>,
    Json(req): Json<AdminDebitRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if req.amount_sats == 0 {
        return Err((StatusCode::BAD_REQUEST, "Amount must be positive".to_string()));
    }

    let payments = state.payments_state.as_ref()
        .ok_or_else(|| (StatusCode::SERVICE_UNAVAILABLE, "Payments not enabled".to_string()))?;

    let amount_msat = (req.amount_sats as i64) * 1000;
    let ref_id = format!("admin-debit-{}", chrono::Utc::now().timestamp_millis());
    let reason = req.reason.as_deref().unwrap_or("admin_debit");

    let new_balance = payments.balance_service
        .debit(req.user_id, amount_msat, reason, &ref_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tracing::info!(
        admin_action = "debit",
        user_id = req.user_id,
        amount_msat = amount_msat,
        new_balance_msat = new_balance,
        reason = reason,
        "Admin manual debit"
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "user_id": req.user_id,
        "debited_sats": req.amount_sats,
        "new_balance_msat": new_balance,
        "new_balance_sats": new_balance / 1000,
        "reason": reason,
    })))
}

/// GET /api/payments/transactions — List recent transactions (admin audit)
#[axum::debug_handler]
pub async fn list_admin_transactions(
    State(state): State<std::sync::Arc<AppState>>,
) -> Json<Vec<crate::db::BalanceTransactionRow>> {
    let txs = state.db.list_all_transactions(200).await.unwrap_or_default();
    Json(txs)
}

/// GET /api/payments/invoices — List all lightning invoices (admin)
#[axum::debug_handler]
pub async fn list_admin_invoices(
    State(state): State<std::sync::Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Vec<crate::db::LightningInvoiceRow>> {
    let user_id = params.get("user_id").and_then(|v| v.parse::<i64>().ok());
    let invoices = state.db.list_all_lightning_invoices(user_id, 200).await.unwrap_or_default();
    Json(invoices)
}

// ── Model Access Control Admin APIs ───────────────────────────────────

#[derive(serde::Serialize)]
pub struct UserModelPermissionResponse {
    pub id: i64,
    pub user_id: i64,
    pub model: String,
    pub allow: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(serde::Deserialize)]
pub struct CreateUserModelPermissionRequest {
    pub user_id: i64,
    pub model: String,
    pub allow: bool,
}

#[derive(serde::Deserialize)]
pub struct UpdateUserModelPermissionRequest {
    pub allow: Option<bool>,
}

/// GET /api/users/:user_id/models — List model permissions for a user
#[axum::debug_handler]
pub async fn list_user_model_permissions(
    Path(user_id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<Vec<UserModelPermissionResponse>>, (axum::http::StatusCode, String)> {
    let permissions = state.db
        .list_user_model_permissions(user_id)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let responses: Vec<UserModelPermissionResponse> = permissions
        .into_iter()
        .map(|p| UserModelPermissionResponse {
            id: p.id,
            user_id: p.user_id,
            model: p.model,
            allow: p.allow,
            created_at: p.created_at,
            updated_at: p.updated_at,
        })
        .collect();

    Ok(Json(responses))
}

/// POST /api/users/:user_id/models — Create or update model permission for a user
#[axum::debug_handler]
pub async fn create_user_model_permission(
    Path(user_id): Path<i64>,
    State(state): State<std::sync::Arc<AppState>>,
    Json(request): Json<CreateUserModelPermissionRequest>,
) -> Result<Json<UserModelPermissionResponse>, (axum::http::StatusCode, String)> {
    // Verify user exists
    state.db.get_user_by_id(user_id).await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Upsert permission (INSERT OR REPLACE)
    let permission = sqlx::query_as::<_, crate::db::UserModelPermission>(
        "INSERT INTO user_model_permissions (user_id, model, allow) VALUES (?, ?, ?)
         ON CONFLICT(user_id, model) DO UPDATE SET allow = excluded.allow, updated_at = CURRENT_TIMESTAMP
         RETURNING *"
    )
    .bind(request.user_id)
    .bind(request.model)
    .bind(request.allow)
    .fetch_one(&state.db.pool)
    .await
    .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(UserModelPermissionResponse {
        id: permission.id,
        user_id: permission.user_id,
        model: permission.model,
        allow: permission.allow,
        created_at: permission.created_at,
        updated_at: permission.updated_at,
    }))
}

/// DELETE /api/users/:user_id/models/:model — Remove model permission for a user
#[axum::debug_handler]
pub async fn delete_user_model_permission(
    Path((user_id, model)): Path<(i64, String)>,
    State(state): State<std::sync::Arc<AppState>>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let deleted = state.db
        .delete_user_model_permission(user_id, &model)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((axum::http::StatusCode::NOT_FOUND, "Permission not found".to_string()));
    }

    Ok(Json(serde_json::json!({
        "message": "Permission deleted successfully",
        "user_id": user_id,
        "model": model
    })))
}
