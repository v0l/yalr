use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SESSION_HEADER: &str = "x-session-id";
const IDLE_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_SESSIONS: usize = 50_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionKey(String);

impl SessionKey {
    pub fn new(user_id: Option<i64>, model: &str, session: Option<&str>) -> Option<Self> {
        let session = session.map(str::trim).filter(|s| !s.is_empty())?;
        Some(Self(format!("{}\u{0}{model}\u{0}{session}", user_id.unwrap_or_default())))
    }
}

struct Pin {
    provider: String,
    last_used: Instant,
}

#[derive(Clone, Default)]
pub struct StickySessions {
    pins: Arc<Mutex<HashMap<SessionKey, Pin>>>,
}

impl StickySessions {
    pub fn provider_for(&self, key: &SessionKey) -> Option<String> {
        let mut pins = self.pins.lock().unwrap();
        match pins.get(key) {
            Some(pin) if pin.last_used.elapsed() < IDLE_TTL => Some(pin.provider.clone()),
            Some(_) => {
                pins.remove(key);
                None
            }
            None => None,
        }
    }

    pub fn pin(&self, key: &SessionKey, provider: &str) {
        let mut pins = self.pins.lock().unwrap();
        if pins.len() >= MAX_SESSIONS && !pins.contains_key(key) {
            pins.retain(|_, pin| pin.last_used.elapsed() < IDLE_TTL);
            if pins.len() >= MAX_SESSIONS {
                if let Some(oldest) = pins.iter().min_by_key(|(_, pin)| pin.last_used).map(|(k, _)| k.clone()) {
                    pins.remove(&oldest);
                }
            }
        }
        pins.insert(key.clone(), Pin { provider: provider.to_string(), last_used: Instant::now() });
    }

    pub fn move_pinned_first<T>(&self, key: Option<&SessionKey>, candidates: &mut [T], name: impl Fn(&T) -> &str) {
        let Some(pinned) = key.and_then(|k| self.provider_for(k)) else {
            return;
        };
        if let Some(idx) = candidates.iter().position(|c| name(c) == pinned) {
            candidates[..=idx].rotate_right(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_requires_a_non_empty_session() {
        assert!(SessionKey::new(Some(1), "m", None).is_none());
        assert!(SessionKey::new(Some(1), "m", Some("  ")).is_none());
        assert_ne!(SessionKey::new(Some(1), "m", Some("s")), SessionKey::new(Some(2), "m", Some("s")));
        assert_ne!(SessionKey::new(Some(1), "a", Some("s")), SessionKey::new(Some(1), "b", Some("s")));
    }

    #[test]
    fn pinned_provider_moves_to_front_keeping_the_rest_in_order() {
        let sticky = StickySessions::default();
        let key = SessionKey::new(Some(1), "m", Some("s")).unwrap();
        sticky.pin(&key, "c");
        let mut order = vec!["a", "b", "c", "d"];
        sticky.move_pinned_first(Some(&key), &mut order, |s| s);
        assert_eq!(order, vec!["c", "a", "b", "d"]);
    }

    #[test]
    fn pin_to_a_missing_provider_leaves_order_alone() {
        let sticky = StickySessions::default();
        let key = SessionKey::new(Some(1), "m", Some("s")).unwrap();
        sticky.pin(&key, "gone");
        let mut order = vec!["a", "b"];
        sticky.move_pinned_first(Some(&key), &mut order, |s| s);
        assert_eq!(order, vec!["a", "b"]);
    }

    #[test]
    fn expired_pin_is_dropped() {
        let sticky = StickySessions::default();
        let key = SessionKey::new(Some(1), "m", Some("s")).unwrap();
        sticky.pin(&key, "a");
        sticky.pins.lock().unwrap().get_mut(&key).unwrap().last_used = Instant::now() - IDLE_TTL;
        assert_eq!(sticky.provider_for(&key), None);
    }

    mod routing {
        use crate::db::Database;
        use crate::metrics::{MetricsStore, MetricsUser};
        use crate::providers::{ChatRequest, ChatResponse, Provider};
        use crate::router::engine::Router;
        use crate::ProviderError;
        use async_trait::async_trait;
        use futures::StreamExt;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        struct Backend {
            name: String,
            calls: AtomicUsize,
            failing: AtomicBool,
        }

        fn response() -> ChatResponse {
            serde_json::from_str(
                r#"{"id":"c","object":"chat.completion","created":1,"model":"m",
                "choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"ok"}}]}"#,
            )
            .unwrap()
        }

        fn chunk() -> crate::StreamingChunk {
            serde_json::from_str(
                r#"{"id":"c","object":"chat.completion.chunk","created":1,"model":"m",
                "choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            )
            .unwrap()
        }

        #[async_trait]
        impl Provider for Backend {
            fn name(&self) -> &str {
                &self.name
            }
            fn slug(&self) -> &str {
                &self.name
            }
            async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
                Ok(vec![])
            }
            async fn chat_completions(&self, _request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if self.failing.load(Ordering::SeqCst) {
                    return Err(ProviderError::ServerError { message: "down".into(), status_code: Some(503) });
                }
                Ok(response())
            }
            fn chat_completions_stream(
                &self,
                _request: &ChatRequest,
            ) -> Result<futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>, ProviderError>
            {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(Box::pin(futures::stream::once(async { Ok(chunk()) })))
            }
            async fn health_check(&self) -> Result<bool, ProviderError> {
                Ok(true)
            }
        }

        async fn pool(names: &[&str]) -> (Router, Vec<Arc<Backend>>) {
            let backends: Vec<Arc<Backend>> = names
                .iter()
                .map(|n| Arc::new(Backend { name: n.to_string(), calls: AtomicUsize::new(0), failing: AtomicBool::new(false) }))
                .collect();
            let db = Arc::new(Database::test().await);
            let router = Router::new(MetricsStore::new(100), db);
            router
                .register_route("pool", backends.iter().map(|b| b.clone() as Arc<dyn Provider>).collect())
                .await;
            (router, backends)
        }

        fn user(id: i64) -> Option<MetricsUser> {
            Some(MetricsUser { id: Some(id), name: None, api_key_id: None, api_key_name: None })
        }

        fn request(session_field: Option<&str>, header: Option<&str>) -> ChatRequest {
            let cache_key = session_field.map(|s| format!(r#","prompt_cache_key":"{s}""#)).unwrap_or_default();
            let request: ChatRequest = serde_json::from_str(&format!(
                r#"{{"model":"pool","messages":[{{"role":"user","content":"hi"}}]{cache_key}}}"#
            ))
            .unwrap();
            request.with_session_header(header.map(str::to_string))
        }

        fn calls(backends: &[Arc<Backend>]) -> Vec<usize> {
            backends.iter().map(|b| b.calls.load(Ordering::SeqCst)).collect()
        }

        #[tokio::test]
        async fn session_header_keeps_round_robin_on_one_provider() {
            let (router, backends) = pool(&["a", "b", "c"]).await;
            for _ in 0..6 {
                router.chat_completions(&request(None, Some("s1")), user(1)).await.unwrap();
            }
            let counts = calls(&backends);
            assert_eq!(counts.iter().filter(|&&c| c == 6).count(), 1, "{counts:?}");
        }

        #[tokio::test]
        async fn prompt_cache_key_sticks_for_streams() {
            let (router, backends) = pool(&["a", "b", "c"]).await;
            for _ in 0..4 {
                let mut stream = router.chat_completions_stream(&request(Some("conv"), None), user(1)).await.unwrap();
                while let Some(chunk) = stream.next().await {
                    chunk.unwrap();
                }
            }
            let counts = calls(&backends);
            assert_eq!(counts.iter().filter(|&&c| c == 4).count(), 1, "{counts:?}");
        }

        #[tokio::test]
        async fn without_a_session_requests_still_rotate() {
            let (router, backends) = pool(&["a", "b", "c"]).await;
            for _ in 0..6 {
                router.chat_completions(&request(None, None), user(1)).await.unwrap();
            }
            assert_eq!(calls(&backends), vec![2, 2, 2]);
        }

        #[tokio::test]
        async fn sessions_are_scoped_per_user() {
            let (router, backends) = pool(&["a", "b"]).await;
            router.chat_completions(&request(None, Some("same")), user(1)).await.unwrap();
            router.chat_completions(&request(None, Some("same")), user(2)).await.unwrap();
            assert_eq!(calls(&backends), vec![1, 1]);
        }

        #[tokio::test]
        async fn failed_pin_moves_the_session_to_the_provider_that_answered() {
            let (router, backends) = pool(&["a", "b"]).await;
            router.chat_completions(&request(None, Some("s")), user(1)).await.unwrap();
            let pinned = backends.iter().position(|b| b.calls.load(Ordering::SeqCst) == 1).unwrap();
            let other = 1 - pinned;
            backends[pinned].failing.store(true, Ordering::SeqCst);
            router.chat_completions(&request(None, Some("s")), user(1)).await.unwrap();
            backends[pinned].failing.store(false, Ordering::SeqCst);
            for _ in 0..3 {
                router.chat_completions(&request(None, Some("s")), user(1)).await.unwrap();
            }
            assert_eq!(backends[pinned].calls.load(Ordering::SeqCst), 2);
            assert_eq!(backends[other].calls.load(Ordering::SeqCst), 4);
        }

        #[tokio::test]
        async fn pin_to_a_provider_in_backoff_is_not_forced_first() {
            let (router, backends) = pool(&["a", "b"]).await;
            router.chat_completions(&request(None, Some("s")), user(1)).await.unwrap();
            let pinned = backends.iter().position(|b| b.calls.load(Ordering::SeqCst) == 1).unwrap();
            let name = backends[pinned].name.clone();
            for _ in 0..5 {
                router.metrics_store.record(crate::metrics::ProviderMetrics {
                    provider: name.clone(),
                    model: "pool".into(),
                    timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
                    event: crate::metrics::MetricsEvent::Failure(crate::metrics::FailureDetails {
                        error_type: crate::metrics::ErrorType::Other,
                        error_code: None,
                        error_message: "down".into(),
                        retry_after_ms: None,
                        status_code: None,
                    }),
                    user: None,
                }).await;
            }
            let key = super::super::SessionKey::new(Some(1), "pool", Some("s"));
            let candidates = router.collect_session_candidates("pool", key.as_ref()).await;
            assert_ne!(candidates[0].0.name(), name);
        }
    }
}
