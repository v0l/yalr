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
    pub amount_msat: i64,
    pub detail: Option<String>,
    pub created_at: crate::db::Timestamp,
}

#[derive(Serialize)]
pub struct Account {
    pub balance_msat: i64,
    pub lifetime_deposited_msat: i64,
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
                out[i].amount_msat += row.amount_msat;
                continue;
            }
            by_request.insert(rid.clone(), out.len());
            out.push(HistoryEntry {
                id: row.id,
                kind: k,
                amount_msat: row.amount_msat,
                detail: models.get(&rid).cloned(),
                created_at: row.created_at,
            });
            continue;
        }
        let detail = match row.transaction_type.as_str() {
            "refund_fee" => Some("lightning routing fee".to_string()),
            "refund" => Some("released reservation".to_string()),
            "refund_reversal" => Some("withdrawal failed, returned".to_string()),
            "deposit" | "usage" | "reserve" | "overage" => None,
            other => Some(other.to_string()),
        };
        out.push(HistoryEntry {
            id: row.id,
            kind: k,
            amount_msat: row.amount_msat,
            detail,
            created_at: row.created_at,
        });
    }
    out.retain(|e| e.amount_msat != 0);
    out.reverse();
    out
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
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
        balance_msat: balance.as_ref().map_or(0, |b| b.balance_msat),
        lifetime_deposited_msat: balance.as_ref().map_or(0, |b| b.lifetime_deposited_msat),
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
        assert_eq!((h[0].kind, h[0].amount_msat, h[0].detail.as_deref()), ("usage", -1000, Some("qwen")));
        assert_eq!((h[1].kind, h[1].amount_msat), ("deposit", 100_000));
    }

    #[test]
    fn a_fully_released_request_disappears() {
        let rows = vec![row(2, 7000, "refund", "req-b"), row(1, -7000, "reserve", "req-b")];
        assert!(fold_history(rows, &HashMap::new()).is_empty());
    }

    #[test]
    fn withdrawals_and_fees_are_labelled() {
        let rows = vec![row(2, -3, "refund_fee", "refund-1"), row(1, -5000, "refund", "refund-1")];
        let h = fold_history(rows, &HashMap::new());
        assert_eq!(h[0].kind, "withdrawal");
        assert_eq!(h[0].detail.as_deref(), Some("lightning routing fee"));
    }
}
