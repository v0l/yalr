// ── E2E Mock LLM Server ───────────────────────────────────────────────────
//
// A minimal OpenAI-compatible server that responds with fixed chat completion
// responses containing usage data. Used by e2e tests to verify billing.
//
// Returns: 50 prompt tokens, 150 completion tokens, model echoes request.
//
// Build and run:
//   cargo build --bin e2e-mock-llm
//   ./target/debug/e2e-mock-llm    # listens on 0.0.0.0:4000

use axum::{routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:4000").await.unwrap();
    eprintln!("Mock LLM listening on 0.0.0.0:4000");
    axum::serve(listener, app).await.unwrap();
}

async fn list_models() -> Json<serde_json::Value> {
    Json(json!({
        "object": "list",
        "data": [
            {"id": "mock-model", "object": "model"}
        ]
    }))
}

#[derive(Deserialize)]
struct ChatRequest {
    model: String,
    #[serde(default)]
    stream: bool,
    max_tokens: Option<u32>,
    messages: Option<Vec<serde_json::Value>>,
}

#[derive(Serialize)]
struct ChatResponse {
    id: String,
    object: String,
    created: u64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Serialize)]
struct Choice {
    index: u32,
    message: ChoiceMessage,
    finish_reason: String,
}

#[derive(Serialize)]
struct ChoiceMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

async fn chat_completions(Json(req): Json<ChatRequest>) -> Json<ChatResponse> {
    Json(ChatResponse {
        id: format!("mock-{}", uuid::Uuid::new_v4()),
        object: "chat.completion".to_string(),
        created: chrono::Utc::now().timestamp() as u64,
        model: req.model.clone(),
        choices: vec![Choice {
            index: 0,
            message: ChoiceMessage {
                role: "assistant".to_string(),
                content: format!("Mock response to: {}", req.messages
                    .and_then(|m| m.last()
                        .and_then(|msg| msg["content"].as_str())
                        .map(|c| c.to_string())
                        .or_else(|| Some("...".to_string()))
                    ).unwrap_or_default()),
            },
            finish_reason: "stop".to_string(),
        }],
        usage: Usage {
            prompt_tokens: 50,
            completion_tokens: 150,
            total_tokens: 200,
        },
    })
}
