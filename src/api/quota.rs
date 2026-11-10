use std::collections::HashSet;
use std::sync::Arc;

use axum::{extract::State, Extension, Json};

use crate::auth::admin::AuthenticatedUser;
use crate::quota_report::{build_report, ProviderQuota, QuotaReport};
use crate::state::AppState;

pub async fn routable_quotas(state: &AppState, authenticated: &AuthenticatedUser) -> Vec<ProviderQuota> {
    let router = &state.config.router;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for model in router.routed_models().await {
        let allowed = crate::api::shared::check_model_access(state, authenticated.user.id, &model).await.is_ok();
        if !allowed {
            continue;
        }
        for (provider, _) in router.candidate_backends(&model).await {
            if !seen.insert(provider.name().to_string()) {
                continue;
            }
            let quotas = state.metrics_store.get_quota(provider.name()).await.unwrap_or_default();
            out.push(ProviderQuota { label: provider.name().to_string(), quotas });
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

pub async fn get_quota(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedUser>,
) -> Json<QuotaReport> {
    let providers = routable_quotas(&state, &authenticated).await;
    let now_ms = chrono::Utc::now().timestamp_millis();
    Json(build_report(providers, authenticated.user.is_admin, now_ms))
}

#[cfg(test)]
mod tests {
    use crate::api::server::create_test_app;
    use crate::db::{Database, NewApiKey};
    use crate::metrics::MetricsStore;
    use crate::providers::provider_trait::QuotaSnapshot;
    use crate::providers::OpenAiProvider;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    const ADMIN_KEY: &str = "sk-admin-quota";
    const PLAIN_KEY: &str = "sk-plain-quota";

    fn snap(window: &str, used: f32, status: &str, resets_at: i64) -> QuotaSnapshot {
        QuotaSnapshot {
            window: Some(window.into()),
            used_pct: Some(used),
            status: Some(status.into()),
            resets_at: Some(resets_at),
            ..Default::default()
        }
    }

    async fn setup(tenx_blocked: bool, fivex_blocked: bool) -> (Arc<AppState>, i64) {
        let db = Arc::new(Database::test().await);
        let metrics_store = Arc::new(MetricsStore::new(100));
        let router = Arc::new(crate::router::engine::Router::new((*metrics_store).clone(), db.clone()));
        let tenx = Arc::new(OpenAiProvider::new("claude-max-10x", Some("tenx"), "http://127.0.0.1:1/v1", Some("k")));
        let fivex = Arc::new(OpenAiProvider::new("claude-max-5x", Some("fivex"), "http://127.0.0.1:1/v1", Some("k")));
        router.add_provider(tenx.clone()).await;
        router.add_provider(fivex.clone()).await;
        router.register_route("claude", vec![tenx, fivex]).await;
        let now = chrono::Utc::now().timestamp_millis();
        let hour = now + 3_600_000;
        let week = now + 200_000_000;
        let far = now + 387_042_000;
        let ten = if tenx_blocked {
            vec![snap("5h", 0.0, "allowed", hour), snap("7d", 100.0, "rejected", far)]
        } else {
            vec![snap("5h", 10.0, "allowed", hour), snap("7d", 40.0, "allowed", week)]
        };
        let five = if fivex_blocked {
            vec![snap("5h", 100.0, "rejected", now + 1_800_000), snap("7d", 17.0, "allowed", week)]
        } else {
            vec![snap("5h", 2.0, "allowed", hour), snap("7d", 17.0, "allowed", week)]
        };
        metrics_store.emitter().emit_quota("claude-max-10x", ten, None);
        metrics_store.emitter().emit_quota("claude-max-5x", five, None);
        let state = Arc::new(AppState {
            config: crate::config::AppConfig {
                db: db.clone(),
                router,
                auth_config: Default::default(),
                payments_config: None,
                discovery_config: None,
                admin_ui_path: String::new(),
                host: String::new(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store,
            session_store: Arc::new(crate::auth::admin::SessionStore::new()),
            db: db.clone(),
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
            limits_cache: Default::default(),
        });
        for (name, key, admin) in [("boss", ADMIN_KEY, true), ("worker", PLAIN_KEY, false)] {
            let user = db.add_test_user(name, admin).await;
            db.create_api_key(NewApiKey {
                key_hash: &crate::auth::api_key::hash_api_key(key),
                name: "k",
                user_id: user.id,
                last_four: "test",
                expires_at: None,
            })
            .await
            .unwrap();
        }
        (state, now)
    }

    async fn fetch(state: &Arc<AppState>, key: &str) -> serde_json::Value {
        let request = Request::builder()
            .uri("/v1/quota")
            .header("authorization", format!("Bearer {key}"))
            .body(Body::empty())
            .unwrap();
        let response = create_test_app(state.clone()).await.oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn admin_key_sees_provider_names_and_the_best_headroom() {
        let (state, _) = setup(true, false).await;
        let body = fetch(&state, ADMIN_KEY).await;
        let windows = body["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 4);
        assert!(windows.iter().any(|w| w["provider"] == "claude-max-5x" && w["window"] == "5h" && w["used_pct"] == 2.0));
        assert_eq!(body["summary"]["window"], "7d");
        assert_eq!(body["summary"]["used_pct"], 17.0);
        assert_eq!(body["summary"]["exhausted"], false);
        assert!(body["summary"]["next_available_at"].is_null());
    }

    #[tokio::test]
    async fn plain_key_gets_the_same_numbers_without_provider_names() {
        let (state, _) = setup(true, false).await;
        let body = fetch(&state, PLAIN_KEY).await;
        let windows = body["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 4);
        assert!(windows.iter().all(|w| w["provider"].is_null()));
        assert_eq!(body["summary"]["used_pct"], 17.0);
        assert!(!body.to_string().contains("claude-max"));
    }

    #[tokio::test]
    async fn exhausted_when_every_provider_is_blocked() {
        let (state, now) = setup(true, true).await;
        let body = fetch(&state, PLAIN_KEY).await;
        assert_eq!(body["summary"]["exhausted"], true);
        assert_eq!(body["summary"]["window"], "5h");
        assert_eq!(body["summary"]["next_available_at"], now + 1_800_000);
    }
}
