use crate::db::Database;
use crate::providers::ChatRequest;

pub async fn with_configured_max_output(db: &Database, request: ChatRequest) -> ChatRequest {
    if request.client_max_tokens().is_some() {
        return request;
    }
    let configured = db
        .get_model_pricing(&request.model)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.max_output_tokens)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0);
    request.with_model_max_output(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(body: serde_json::Value) -> ChatRequest {
        ChatRequest::from_value(body).unwrap()
    }

    #[tokio::test]
    async fn pricing_row_sets_the_maximum_only_when_the_client_sent_none() {
        let db = Database::test().await;
        db.price_model("any-model", true).await;
        db.execute_raw("UPDATE model_pricing SET max_output_tokens = 128000 WHERE model_name = 'any-model'")
            .await
            .unwrap();

        let bare = serde_json::json!({"model": "any-model", "messages": [{"role": "user", "content": "hi"}]});
        let request = with_configured_max_output(&db, chat(bare.clone())).await;
        assert_eq!(request.required_max_tokens(Some(64_000)), 128_000);

        let mut asked = bare.clone();
        asked["max_tokens"] = 20.into();
        let request = with_configured_max_output(&db, chat(asked)).await;
        assert_eq!(request.required_max_tokens(Some(64_000)), 20);

        let mut other = bare;
        other["model"] = "unpriced-model".into();
        let request = with_configured_max_output(&db, chat(other)).await;
        assert_eq!(request.required_max_tokens(Some(64_000)), 64_000);
        assert_eq!(request.required_max_tokens(None), 128_000);
    }
}
