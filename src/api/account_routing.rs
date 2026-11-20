use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};

use crate::auth::admin::UserExtractor;
use crate::db::RoutingPreferences;
use crate::router::preference::Preference;
use crate::state::AppState;

type ApiResult<T> = Result<Json<T>, (StatusCode, String)>;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ModelPreference {
    pub model: String,
    pub preference: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RoutingSettings {
    #[serde(default = "balanced")]
    pub default: String,
    #[serde(default)]
    pub models: Vec<ModelPreference>,
}

fn balanced() -> String {
    Preference::Balanced.as_str().to_string()
}

fn bad_request(message: String) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, message)
}

fn internal(e: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn checked(raw: &str) -> Result<String, (StatusCode, String)> {
    Preference::from_setting(raw)
        .map(|p| p.as_str().to_string())
        .ok_or_else(|| bad_request(format!("Unknown preference '{raw}', use balanced, cheapest or fastest")))
}

pub async fn get(State(state): State<Arc<AppState>>, user: UserExtractor) -> ApiResult<RoutingSettings> {
    let stored = state.db.routing_preferences(user.0.user.id).await.map_err(internal)?;
    Ok(Json(RoutingSettings {
        default: stored.default.unwrap_or_else(balanced),
        models: stored.models.into_iter().map(|(model, preference)| ModelPreference { model, preference }).collect(),
    }))
}

pub async fn put(
    State(state): State<Arc<AppState>>,
    user: UserExtractor,
    Json(settings): Json<RoutingSettings>,
) -> ApiResult<RoutingSettings> {
    let default = checked(&settings.default)?;
    let mut models: Vec<(String, String)> = Vec::new();
    for entry in &settings.models {
        let model = entry.model.trim();
        if model.is_empty() {
            return Err(bad_request("model is required".into()));
        }
        if !models.iter().any(|(m, _)| m.eq_ignore_ascii_case(model)) {
            models.push((model.to_string(), checked(&entry.preference)?));
        }
    }
    let stored = RoutingPreferences { default: (default != "balanced").then_some(default), models };
    state.db.replace_routing_preferences(user.0.user.id, &stored).await.map_err(internal)?;
    get(State(state), user).await
}

#[cfg(test)]
mod tests {
    use crate::api::tenant_tests::tenancy;

    #[tokio::test]
    async fn users_keep_their_own_default_and_per_model_overrides() {
        let t = tenancy().await;
        let (status, body) = t.get(&t.alice, "/api/account/routing").await;
        assert_eq!((status, body["default"].as_str(), body["models"].as_array().map(Vec::len)), (200, Some("balanced"), Some(0)));
        let (status, body) = t
            .call(
                &t.alice,
                "PUT",
                "/api/account/routing",
                r#"{"default":"cheapest","models":[{"model":"acme/m-1","preference":"fastest"}]}"#,
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["default"], "cheapest");
        assert_eq!(body["models"][0]["model"], "acme/m-1");
        let (_, bob) = t.get(&t.bob, "/api/account/routing").await;
        assert_eq!(bob["default"], "balanced");
        let (status, _) = t.call(&t.alice, "PUT", "/api/account/routing", r#"{"default":"vibes"}"#).await;
        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn the_owner_sets_and_clears_a_provider_concurrency_limit() {
        let t = tenancy().await;
        let slug = t.alice_provider().await;
        let (status, body) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"max_concurrency":3}"#).await;
        assert_eq!((status, body["max_concurrency"].as_i64()), (200, Some(3)), "{body}");
        let (status, body) = t.call(&t.alice, "PUT", &format!("/api/providers/{slug}"), r#"{"max_concurrency":null}"#).await;
        assert_eq!((status, body["max_concurrency"].is_null()), (200, true), "{body}");
    }

    #[tokio::test]
    async fn the_owner_sets_a_ttft_deadline_and_a_thinking_level_map() {
        let t = tenancy().await;
        let slug = t.alice_provider().await;
        let uri = format!("/api/providers/{slug}");
        let (status, body) = t
            .call(&t.alice, "PUT", &uri, r#"{"ttft_deadline_ms":1500,"reasoning_effort_map":{"xhigh":"high"}}"#)
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ttft_deadline_ms"], 1500);
        assert_eq!(body["reasoning_effort_map"]["xhigh"], "high");

        let (status, body) = t.call(&t.alice, "PUT", &uri, r#"{"reasoning_effort_map":{"xhigh":3}}"#).await;
        assert_eq!(status, 400, "{body}");

        let (status, body) = t.call(&t.alice, "PUT", &uri, r#"{"ttft_deadline_ms":null,"reasoning_effort_map":null}"#).await;
        assert_eq!(status, 200, "{body}");
        assert!(body["ttft_deadline_ms"].is_null());
        assert!(body.get("reasoning_effort_map").is_none());
    }
}
