use crate::payments::biller::TokenUsage;
use crate::state::AppState;

pub async fn record(
    state: &AppState,
    api_key_id: Option<i64>,
    model: &str,
    usage: TokenUsage,
    billed_msat: i64,
) {
    let Some(api_key_id) = api_key_id else {
        return;
    };
    let db = state.db.clone();
    let model = model.to_string();
    tokio::spawn(async move {
        if let Err(e) = db.record_api_key_usage(api_key_id, &model, usage, billed_msat).await {
            tracing::error!(error = %e, api_key_id, "Failed to record api key usage");
        }
    });
}
