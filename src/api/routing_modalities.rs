use crate::state::AppState;

pub(super) struct RequestedModalities {
    input: Option<Option<Vec<crate::router::Modality>>>,
    output: Option<Option<Vec<crate::router::Modality>>>,
}

impl RequestedModalities {
    pub(super) fn parse(
        input: Option<&[String]>,
        output: Option<&[String]>,
    ) -> Result<Self, (axum::http::StatusCode, String)> {
        let parse = |names: &[String]| {
            crate::db::DeclaredModalities::parse_list(names)
                .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e))
        };
        Ok(Self {
            input: input.map(parse).transpose()?,
            output: output.map(parse).transpose()?,
        })
    }

    pub(super) fn apply(self, current: crate::db::DeclaredModalities) -> crate::db::DeclaredModalities {
        crate::db::DeclaredModalities {
            input: self.input.unwrap_or(current.input),
            output: self.output.unwrap_or(current.output),
        }
    }
}

pub(super) async fn store_modalities(
    state: &AppState,
    config: crate::db::RoutingConfig,
    declared: &crate::db::DeclaredModalities,
) -> Result<crate::db::RoutingConfig, (axum::http::StatusCode, String)> {
    if config.declared_modalities() == *declared {
        return Ok(config);
    }
    let internal = |e: sqlx::Error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    state
        .config
        .db
        .set_routing_config_modalities(config.id, declared)
        .await
        .map_err(internal)?;
    state
        .config
        .db
        .get_routing_config(config.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "Routing config not found".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use crate::api::server::create_test_app;
    use crate::auth::admin::SessionStore;
    use crate::db::{Database, DeclaredModalities, NewUser, UserType};
    use crate::metrics::MetricsStore;
    use crate::router::Modality;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::util::ServiceExt;

    #[test]
    fn absent_lists_keep_current_and_empty_lists_clear() {
        let current = DeclaredModalities {
            input: Some(vec![Modality::Text]),
            output: Some(vec![Modality::Image]),
        };
        let kept = RequestedModalities::parse(None, None).unwrap().apply(current.clone());
        assert_eq!(kept, current);

        let changed = RequestedModalities::parse(Some(&[]), Some(&["audio".to_string()]))
            .unwrap()
            .apply(current);
        assert_eq!(changed.input, None);
        assert_eq!(changed.output, Some(vec![Modality::Audio]));

        let err = RequestedModalities::parse(Some(&["smell".to_string()]), None).err().unwrap();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
    }

    async fn app() -> (axum::Router, String) {
        let db = Database::new("sqlite::memory:").await.unwrap();
        let metrics_store = MetricsStore::new(100);
        let router = Arc::new(crate::router::engine::Router::new(
            metrics_store.clone(),
            Arc::new(db.clone()),
        ));
        let session_store = Arc::new(SessionStore::new());
        let state = Arc::new(AppState {
            config: crate::config::AppConfig {
                db: Arc::new(db.clone()),
                router,
                auth_config: Default::default(),
                payments_config: None,
                admin_ui_path: String::new(),
                host: "127.0.0.1".to_string(),
                port: 0,
            },
            metrics_emitter: metrics_store.emitter().clone(),
            metrics_store: metrics_store.into(),
            session_store: session_store.clone(),
            db: Arc::new(db),
            payments_state: None,
            oauth_pending: Default::default(),
            model_cache: Default::default(),
            modality_cache: Default::default(),
        });
        state
            .db
            .create_user(NewUser {
                username: Some("admin"),
                password_hash: Some("x"),
                external_id: None,
                user_type: UserType::Internal,
                is_admin: true,
            })
            .await
            .unwrap();
        let token = session_store.create("admin", true, 86400).await;
        (create_test_app(state).await, token)
    }

    async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: &str) -> (u16, String) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .method(method)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        (status, String::from_utf8_lossy(&body).to_string())
    }

    #[tokio::test]
    async fn modalities_round_trip_through_the_admin_api() {
        let (app, token) = app().await;

        let (status, body) = send(
            &app,
            "POST",
            "/api/routing-configs",
            &token,
            r#"{"name":"painter","strategy":"round_robin","health_check_enabled":false,"health_check_interval_seconds":30,"health_check_timeout_seconds":5,"output_modalities":["image"]}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(r#""output_modalities":["image"]"#), "{body}");
        assert!(!body.contains("input_modalities"), "{body}");

        let (status, body) = send(
            &app,
            "PUT",
            "/api/routing-configs/1",
            &token,
            r#"{"input_modalities":["text","image"]}"#,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        assert!(body.contains(r#""input_modalities":["text","image"]"#), "{body}");
        assert!(body.contains(r#""output_modalities":["image"]"#), "{body}");

        let (status, _) = send(
            &app,
            "PUT",
            "/api/routing-configs/1",
            &token,
            r#"{"name":"renamed","output_modalities":["hologram"]}"#,
        )
        .await;
        assert_eq!(status, 400);
        let (_, body) = send(&app, "GET", "/api/routing-configs", &token, "").await;
        assert!(body.contains(r#""name":"painter""#), "a rejected update must not half-apply: {body}");
    }
}
