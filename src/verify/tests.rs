use std::sync::Arc;

use async_trait::async_trait;

use super::probes::expected_output;
use super::*;
use crate::db::{Database, FiatPrices, NewModelPricing, NewProvider, APPROVED, PENDING, SUSPENDED};
use crate::providers::{ChatRequest, Provider, ProviderError};
use crate::router::engine::Router;
use crate::metrics::MetricsStore;

const MODEL: &str = "qwen/qwen3.8-27b";

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Faithful,
    Drifting,
    OtherModel,
    CachedFixed,
    Down,
}

struct Fake {
    slug: String,
    behaviour: Behaviour,
    logprobs: bool,
    listed: Option<&'static str>,
}

fn filler(prompt: &str, salt: &str) -> Vec<String> {
    let seed: u32 = prompt.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    (0..32).map(|i| format!("w{}{salt}", (seed.wrapping_add(i * 13)) % 997)).collect()
}

fn prompt_of(request: &ChatRequest) -> String {
    let value = serde_json::to_value(request.inner()).unwrap();
    value["messages"][0]["content"].as_str().unwrap().to_string()
}

impl Fake {
    fn new(slug: &str, behaviour: Behaviour) -> Arc<Self> {
        Arc::new(Self { slug: slug.to_string(), behaviour, logprobs: true, listed: None })
    }

    fn pieces(&self, prompt: &str) -> Vec<String> {
        let fixed = fixed_probes().iter().any(|p| p.prompt == prompt);
        match self.behaviour {
            Behaviour::OtherModel => match expected_output(prompt) {
                Some(_) => vec!["banana".to_string()],
                None => filler(prompt, "x"),
            },
            Behaviour::CachedFixed if !fixed => vec!["cached".to_string()],
            _ => match expected_output(prompt) {
                Some(answer) => vec![answer],
                None => {
                    let mut pieces = filler(prompt, "");
                    if self.behaviour == Behaviour::Drifting {
                        for piece in pieces.iter_mut().skip(20) {
                            piece.push('!');
                        }
                    }
                    pieces
                }
            },
        }
    }

    fn prompt_tokens(&self, prompt: &str) -> u32 {
        let base = prompt.split_whitespace().count() as u32 + 3;
        if self.behaviour == Behaviour::OtherModel { base * 2 + 1 } else { base }
    }
}

#[async_trait]
impl Provider for Fake {
    fn name(&self) -> &str {
        &self.slug
    }
    fn slug(&self) -> &str {
        &self.slug
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![crate::Model { id: self.listed.unwrap_or(MODEL).into(), object: "model".into(), created: 0, owned_by: "x".into() }])
    }
    async fn chat_completions(&self, request: &ChatRequest) -> Result<crate::ChatResponse, ProviderError> {
        if self.behaviour == Behaviour::Down {
            return Err(ProviderError::ServerError { message: "down".into(), status_code: Some(503) });
        }
        let value = serde_json::to_value(request.inner()).unwrap();
        if let Some(listed) = self.listed {
            assert_eq!(value["model"], listed);
        }
        let wants_logprobs = value["logprobs"] == true && value["top_logprobs"] == 5;
        let prompt = prompt_of(request);
        let pieces = self.pieces(&prompt);
        let tokens: Vec<serde_json::Value> = pieces
            .iter()
            .enumerate()
            .map(|(i, piece)| {
                serde_json::json!({
                    "token": piece, "logprob": -0.1 * (i % 3) as f64, "bytes": null,
                    "top_logprobs": [{"token": piece, "logprob": -0.1, "bytes": null}, {"token": "a", "logprob": -2.0, "bytes": null}]
                })
            })
            .collect();
        let mut choice = serde_json::json!({
            "index": 0, "finish_reason": "stop",
            "message": {"role": "assistant", "content": pieces.join(" ")}
        });
        if wants_logprobs && self.logprobs {
            choice["logprobs"] = serde_json::json!({"content": tokens, "refusal": null});
        }
        let pt = self.prompt_tokens(&prompt);
        let body = serde_json::json!({
            "id": "c", "object": "chat.completion", "created": 1, "model": MODEL,
            "choices": [choice],
            "usage": {"prompt_tokens": pt, "completion_tokens": pieces.len(), "total_tokens": pt as usize + pieces.len()}
        });
        Ok(crate::ChatResponse::from_value(body).unwrap())
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>, ProviderError> {
        Err(ProviderError::Unsupported("no stream".into()))
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

struct World {
    db: Arc<Database>,
    router: Arc<Router>,
}

async fn world() -> World {
    let db = Arc::new(Database::test().await);
    let metrics = MetricsStore::new(100);
    let router = Arc::new(Router::new(metrics, db.clone()));
    World { db, router }
}

impl World {
    async fn reference(&self, fake: Arc<Fake>) -> i64 {
        let record = self
            .db
            .create_provider(NewProvider { name: &fake.slug, slug: &fake.slug, base_url: "https://ref.example/v1", api_key: None, provider_type: None })
            .await
            .unwrap();
        self.db.set_provider_reference(record.id, true).await.unwrap();
        self.router.add_provider(fake).await;
        record.id
    }

    async fn user_provider(&self, owner: i64, fake: Arc<Fake>, status: &str) -> i64 {
        let record = self
            .db
            .create_owned_provider(owner, NewProvider { name: &fake.slug, slug: &fake.slug, base_url: "https://u.example/v1", api_key: None, provider_type: None })
            .await
            .unwrap();
        self.db.set_provider_status(record.id, status, None, false).await.unwrap();
        self.db
            .create_model_pricing(NewModelPricing {
                model_name: MODEL,
                is_advertised: true,
                is_free: false,
                price_per_1m_input_sats: None,
                price_per_1m_output_sats: None,
                price_per_1m_cache_read_sats: None,
                price_per_1m_cache_write_sats: None,
                price_per_request_sats: None,
                context_window: None,
                max_output_tokens: None,
                owner_user_id: Some(owner),
                provider_id: Some(record.id),
                currency: "usd",
                fiat: FiatPrices { input: Some(10), output: Some(10), ..Default::default() },
            })
            .await
            .unwrap();
        self.router.add_provider(fake).await;
        record.id
    }

    async fn status(&self, id: i64) -> String {
        self.db.get_provider_by_id(id).await.unwrap().unwrap().status
    }
}

async fn verify(w: &World, id: i64) -> Vec<ModelReport> {
    verify_and_apply(&w.db, &w.router, id).await
}

#[tokio::test]
async fn a_provider_serving_the_same_model_is_approved() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Faithful), PENDING).await;
    let reports = verify(&w, id).await;
    assert_eq!(reports.len(), 1);
    assert!(reports[0].passed, "{}", reports[0].reason);
    assert_eq!(reports[0].scores.tasks_failed, 0);
    assert_eq!(reports[0].scores.prompt_tokens_mismatched, 0);
    assert!(reports[0].scores.mean_logprob_gap.unwrap() < 0.01);
    assert_eq!(w.status(id).await, APPROVED);
    let stored = w.db.latest_verifications(id).await.unwrap();
    assert_eq!(stored.len(), 1);
    assert!(stored[0].passed);
    assert!(stored[0].results.as_deref().unwrap().contains("math-17x23"));
}

#[tokio::test]
async fn late_divergence_inside_tolerance_still_passes() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Drifting), PENDING).await;
    let reports = verify(&w, id).await;
    assert!(reports[0].passed, "{}", reports[0].reason);
    assert!(reports[0].scores.median_prefix >= 16.0);
    assert_eq!(w.status(id).await, APPROVED);
}

#[tokio::test]
async fn a_provider_serving_another_model_fails_on_tokens_and_tasks() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::OtherModel), PENDING).await;
    let reports = verify(&w, id).await;
    assert!(!reports[0].passed);
    assert_eq!(reports[0].outcome, OUTCOME_FAILED);
    assert!(reports[0].scores.prompt_tokens_mismatched > 0);
    assert!(reports[0].scores.tasks_failed > 0);
    assert!(reports[0].reason.contains("prompt token counts"));
    assert_eq!(w.status(id).await, PENDING);
    let row = w.db.get_provider_by_id(id).await.unwrap().unwrap();
    assert!(row.status_reason.unwrap().contains("task probes failed"));
}

#[tokio::test]
async fn cached_fixed_answers_fail_the_random_probes() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::CachedFixed), PENDING).await;
    let reports = verify(&w, id).await;
    assert!(!reports[0].passed);
    let random_failures: Vec<_> = reports[0].results.iter().filter(|r| !r.fixed && r.task_passed == Some(false)).collect();
    assert_eq!(random_failures.len(), crate::verify::settings::get().random_probes);
    assert!(reports[0].results.iter().filter(|r| r.fixed).all(|r| r.task_passed != Some(false)));
    assert_eq!(w.status(id).await, PENDING);
}

#[tokio::test]
async fn random_probes_change_between_runs() {
    let a: Vec<String> = draw_probes(4).into_iter().filter(|p| !p.fixed).map(|p| p.id).collect();
    let differs = (0..20).any(|_| draw_probes(4).into_iter().filter(|p| !p.fixed).map(|p| p.id).collect::<Vec<_>>() != a);
    assert!(differs);
    assert_eq!(draw_probes(4).len(), fixed_probes().len() + 4);
}

#[tokio::test]
async fn a_provider_without_logprobs_is_compared_on_text() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let plain = Arc::new(Fake { slug: "alice-p".into(), behaviour: Behaviour::Faithful, logprobs: false, listed: None });
    let id = w.user_provider(alice.id, plain, PENDING).await;
    let reports = verify(&w, id).await;
    assert!(reports[0].passed, "{}", reports[0].reason);
    assert!(reports[0].scores.mean_logprob_gap.is_none());
}

#[tokio::test]
async fn without_a_reference_the_provider_stays_pending_and_says_why() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Faithful), PENDING).await;
    let reports = verify(&w, id).await;
    assert_eq!(reports[0].outcome, OUTCOME_NO_REFERENCE);
    assert_eq!(w.status(id).await, PENDING);
}

#[tokio::test]
async fn an_unreachable_provider_is_inconclusive_not_a_failure() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Down), APPROVED).await;
    let reports = verify(&w, id).await;
    assert_eq!(reports[0].outcome, OUTCOME_ERROR);
    assert_eq!(w.status(id).await, APPROVED);
}

#[tokio::test]
async fn the_schedule_reverifies_and_suspends_a_provider_that_regressed() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    let bob = w.db.add_test_user("bob", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::OtherModel), APPROVED).await;
    w.router.refresh_full_name_index().await;
    assert_eq!(w.router.full_name_providers(MODEL).await.len(), 2);

    assert_eq!(run_due(&w.db, &w.router).await, 1);
    assert_eq!(w.status(id).await, SUSPENDED);
    assert!(w.db.get_provider_by_id(id).await.unwrap().unwrap().status_reason.unwrap().contains("prompt token"));
    let members = w.router.full_name_providers(MODEL).await;
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].0, "ref");
    let _ = bob;

    assert_eq!(run_due(&w.db, &w.router).await, 0);
    w.db.execute_raw("UPDATE provider_verifications SET created_at = UTC_TIMESTAMP(3) - INTERVAL 2 DAY").await.unwrap();
    assert_eq!(run_due(&w.db, &w.router).await, 1);
}

#[tokio::test]
async fn a_changed_provider_drops_out_of_the_pool_until_it_verifies_again() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Faithful), APPROVED).await;
    w.router.refresh_full_name_index().await;
    let provider = w.db.get_provider_by_id(id).await.unwrap().unwrap();
    mark_changed(&w.db, &w.router, &provider).await;
    assert_eq!(w.status(id).await, PENDING);
    assert_eq!(w.router.full_name_providers(MODEL).await.len(), 1);
    verify(&w, id).await;
    assert_eq!(w.status(id).await, APPROVED);
    assert_eq!(w.router.full_name_providers(MODEL).await.len(), 2);
}

#[tokio::test]
async fn a_hand_decision_is_not_overwritten_by_a_verification_run() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let id = w.user_provider(alice.id, Fake::new("alice-p", Behaviour::Faithful), PENDING).await;
    w.db.set_provider_status(id, SUSPENDED, Some("rejected by admin"), true).await.unwrap();
    verify(&w, id).await;
    assert_eq!(w.status(id).await, SUSPENDED);
    assert_eq!(w.db.latest_verifications(id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_provider_that_serves_the_bare_model_name_is_probed_under_that_name() {
    let w = world().await;
    let alice = w.db.add_test_user("alice", false).await;
    w.reference(Fake::new("ref", Behaviour::Faithful)).await;
    let bare = Arc::new(Fake { slug: "alice-p".into(), behaviour: Behaviour::Faithful, logprobs: true, listed: Some("qwen3.8-27b") });
    let id = w.user_provider(alice.id, bare, PENDING).await;
    let reports = verify(&w, id).await;
    assert!(reports[0].passed, "{}", reports[0].reason);
}
