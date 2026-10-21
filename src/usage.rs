use crate::payments::biller::TokenUsage;
use crate::state::AppState;

pub async fn cost_msat(state: &AppState, model: &str, usage: TokenUsage) -> i64 {
    let Some(payments) = state.payments_state.as_ref() else {
        return 0;
    };
    let pricing = payments.pricing_resolver.resolve(model).await;
    if pricing.is_free {
        0
    } else {
        usage.cost_msat(&pricing)
    }
}

pub async fn record(state: &AppState, api_key_id: Option<i64>, model: &str, usage: TokenUsage) {
    let Some(api_key_id) = api_key_id else {
        return;
    };
    let cost = cost_msat(state, model, usage).await;
    if let Err(e) = state.db.record_api_key_usage(api_key_id, model, usage, cost).await {
        tracing::error!(error = %e, api_key_id, "Failed to record api key usage");
    }
}
