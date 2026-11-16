use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::{Extension, Query, State}, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::{Deserialize, Serialize};

use crate::auth::admin::AuthenticatedUser;
use crate::db::BalanceTransactionRow;
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 500;

#[derive(Deserialize)]
pub struct HistoryQuery {
    limit: Option<i64>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct HistoryEntry {
    pub id: i64,
    pub kind: &'static str,
    pub amount_milli: i64,
    pub currency: String,
    pub detail: Option<String>,
    pub created_at: crate::db::Timestamp,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct CurrencyBalance {
    pub currency: String,
    pub balance_milli: i64,
    pub lifetime_deposited_milli: i64,
}

#[derive(Serialize)]
pub struct Account {
    pub display_name: String,
    pub balance_msat: i64,
    pub lifetime_deposited_msat: i64,
    pub balances: Vec<CurrencyBalance>,
    pub history: Vec<HistoryEntry>,
    pub invoices: Vec<InvoiceEntry>,
}

#[derive(Serialize)]
pub struct InvoiceEntry {
    pub invoice_id: String,
    pub purpose: String,
    pub amount_sats: i64,
    pub status: String,
    pub created_at: crate::db::Timestamp,
    pub paid_at: Option<crate::db::Timestamp>,
}

fn row_amount(row: &BalanceTransactionRow) -> i64 {
    if row.currency == "sats" { row.amount_msat } else { row.amount_milli }
}

fn kind(tx_type: &str) -> &'static str {
    match tx_type {
        "deposit" => "deposit",
        "usage" | "reserve" | "refund" | "overage" => "usage",
        "refund_fee" | "refund_reversal" => "withdrawal",
        _ => "adjustment",
    }
}

pub fn fold_history(rows: Vec<BalanceTransactionRow>, models: &HashMap<String, String>) -> Vec<HistoryEntry> {
    let mut out: Vec<HistoryEntry> = Vec::new();
    let mut by_request: HashMap<String, usize> = HashMap::new();

    for row in rows.into_iter().rev() {
        let k = kind(&row.transaction_type);
        let request = row.reference_id.clone().filter(|r| k == "usage" && r.starts_with("req-"));
        if let Some(rid) = request {
            if let Some(&i) = by_request.get(&rid) {
                out[i].amount_milli += row_amount(&row);
                continue;
            }
            by_request.insert(rid.clone(), out.len());
            out.push(HistoryEntry {
                id: row.id,
                kind: k,
                amount_milli: row_amount(&row),
                currency: row.currency.clone(),
                detail: models.get(&rid).cloned(),
                created_at: row.created_at,
            });
            continue;
        }
        let detail = match row.transaction_type.as_str() {
            "refund_fee" => Some("lightning routing fee".to_string()),
            "refund" => Some("released reservation".to_string()),
            "refund_reversal" => Some("withdrawal failed, returned".to_string()),
            "failover_penalty" => Some("failover: your provider failed, prefill charged to you".to_string()),
            "deposit" | "usage" | "reserve" | "overage" => None,
            other => Some(other.to_string()),
        };
        out.push(HistoryEntry {
            id: row.id,
            kind: k,
            amount_milli: row_amount(&row),
            currency: row.currency.clone(),
            detail,
            created_at: row.created_at,
        });
    }
    out.retain(|e| e.amount_milli != 0);
    out.reverse();
    out
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

async fn all_balances(
    state: &AppState,
    user_id: i64,
    sats: Option<&crate::db::UserBalanceRow>,
) -> Result<Vec<CurrencyBalance>, sqlx::Error> {
    let mut out = vec![CurrencyBalance {
        currency: "sats".into(),
        balance_milli: sats.map_or(0, |b| b.balance_msat),
        lifetime_deposited_milli: sats.map_or(0, |b| b.lifetime_deposited_msat),
    }];
    for (currency, balance_milli, lifetime_deposited_milli) in state.db.list_currency_balances(user_id).await? {
        if currency != "sats" {
            out.push(CurrencyBalance { currency, balance_milli, lifetime_deposited_milli });
        }
    }
    Ok(out)
}

pub async fn account(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let Some(payments) = state.payments_state.as_ref() else {
        return error(StatusCode::NOT_FOUND, "Payments not enabled");
    };
    let user_id = authenticated.user.id;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let balance = match state.db.get_user_balance(user_id).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read balance"),
    };
    let _ = payments;
    let balances = match all_balances(&state, user_id, balance.as_ref()).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read balance"),
    };
    let rows = match state.db.get_user_transactions(user_id, limit * 2).await {
        Ok(r) => r,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "Failed to read history"),
    };
    let models = state
        .db
        .request_models(user_id, rows.iter().filter_map(|r| r.reference_id.clone()).collect())
        .await
        .unwrap_or_default();
    let mut history = fold_history(rows, &models);
    history.truncate(limit as usize);

    let invoices = state
        .db
        .list_account_invoices(user_id, 50)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|i| InvoiceEntry {
            invoice_id: i.public_id,
            purpose: i.purpose,
            amount_sats: i.amount_sats,
            status: i.status,
            created_at: i.created_at,
            paid_at: i.paid_at,
        })
        .collect();

    Json(Account {
        display_name: authenticated.user.shown_name(),
        balance_msat: balance.as_ref().map_or(0, |b| b.balance_msat),
        lifetime_deposited_msat: balance.as_ref().map_or(0, |b| b.lifetime_deposited_msat),
        balances,
        history,
        invoices,
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, amount: i64, tx: &str, reference: &str) -> BalanceTransactionRow {
        BalanceTransactionRow {
            id,
            user_id: 1,
            amount_msat: amount,
            transaction_type: tx.into(),
            reference_id: Some(reference.into()),
            metadata: None,
            currency: "sats".into(),
            amount_milli: 0,
            created_at: chrono::DateTime::from_timestamp(1_792_504_800 + id, 0).unwrap(),
        }
    }

    #[test]
    fn a_request_folds_into_one_line_with_its_net_cost() {
        let rows = vec![
            row(3, 6000, "refund", "req-a"),
            row(2, -7000, "reserve", "req-a"),
            row(1, 100_000, "deposit", "hash"),
        ];
        let models = HashMap::from([("req-a".to_string(), "qwen".to_string())]);
        let h = fold_history(rows, &models);
        assert_eq!(h.len(), 2);
        assert_eq!((h[0].kind, h[0].amount_milli, h[0].detail.as_deref()), ("usage", -1000, Some("qwen")));
        assert_eq!((h[1].kind, h[1].amount_milli), ("deposit", 100_000));
    }

    #[test]
    fn a_fully_released_request_disappears() {
        let rows = vec![row(2, 7000, "refund", "req-b"), row(1, -7000, "reserve", "req-b")];
        assert!(fold_history(rows, &HashMap::new()).is_empty());
    }

    #[test]
    fn a_failover_penalty_is_labelled_for_the_provider_owner() {
        let row = BalanceTransactionRow {
            id: 1,
            user_id: 1,
            amount_msat: -500,
            transaction_type: "failover_penalty".into(),
            reference_id: Some("req-x".into()),
            metadata: None,
            currency: "sats".into(),
            amount_milli: 0,
            created_at: chrono::Utc::now(),
        };
        let h = fold_history(vec![row], &HashMap::new());
        assert_eq!((h[0].kind, h[0].amount_milli), ("adjustment", -500));
        assert!(h[0].detail.as_deref().unwrap().starts_with("failover:"));
    }

    #[test]
    fn withdrawals_and_fees_are_labelled() {
        let rows = vec![row(2, -3, "refund_fee", "refund-1"), row(1, -5000, "refund", "refund-1")];
        let h = fold_history(rows, &HashMap::new());
        assert_eq!(h[0].kind, "withdrawal");
        assert_eq!(h[0].detail.as_deref(), Some("lightning routing fee"));
    }
}
