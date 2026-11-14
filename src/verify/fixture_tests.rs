use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::providers::{ChatRequest, Provider, ProviderError};

struct Recorded {
    name: &'static str,
    responses: HashMap<String, serde_json::Value>,
}

fn load(name: &'static str, body: &str) -> Arc<dyn Provider> {
    Arc::new(Recorded { name, responses: serde_json::from_str(body).unwrap() })
}

fn spark() -> Arc<dyn Provider> {
    load("spark-3.8", include_str!("fixtures/spark-qwen3.8-27b.json"))
}

fn last_user(request: &ChatRequest) -> String {
    let value = serde_json::to_value(request.inner()).unwrap();
    value["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string()
}

#[async_trait]
impl Provider for Recorded {
    fn name(&self) -> &str {
        self.name
    }
    fn slug(&self) -> &str {
        self.name
    }
    async fn list_models(&self) -> Result<Vec<crate::Model>, ProviderError> {
        Ok(vec![])
    }
    async fn chat_completions(&self, request: &ChatRequest) -> Result<crate::ChatResponse, ProviderError> {
        let prompt = last_user(request);
        let probe = fixed_probes().into_iter().chain(pool_probes()).find(|p| p.prompt == prompt).unwrap();
        let mut body = self.responses.get(&probe.id).unwrap_or_else(|| panic!("{} has no {}", self.name, probe.id)).clone();
        let value = serde_json::to_value(request.inner()).unwrap();
        if value["logprobs"] != true {
            body["choices"][0]["logprobs"] = serde_json::Value::Null;
        }
        Ok(crate::ChatResponse::from_value(body).unwrap())
    }
    fn chat_completions_stream(
        &self,
        _request: &ChatRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<crate::StreamingChunk, ProviderError>>, ProviderError> {
        Err(ProviderError::Unsupported("recorded".into()))
    }
    async fn health_check(&self) -> Result<bool, ProviderError> {
        Ok(true)
    }
}

fn draws() -> Vec<Vec<Probe>> {
    let pool = pool_probes();
    let mut out = Vec::new();
    for a in 0..pool.len() {
        for b in a + 1..pool.len() {
            for c in b + 1..pool.len() {
                for d in c + 1..pool.len() {
                    let mut probes = fixed_probes();
                    probes.extend([a, b, c, d].map(|i| pool[i].clone()));
                    out.push(probes);
                }
            }
        }
    }
    out
}

async fn outcomes(candidate: Arc<dyn Provider>, every: usize) -> HashMap<&'static str, usize> {
    let reference = spark();
    let config = VerificationConfig::default();
    let mut counts = HashMap::new();
    for probes in draws().into_iter().step_by(every) {
        let report = run_probes(probes, &candidate, "m", &reference, "m", &config).await;
        *counts.entry(report.outcome).or_default() += 1;
    }
    counts
}

#[tokio::test]
async fn recorded_qwen38_from_another_host_passes_every_draw() {
    let counts = outcomes(load("or-3.8", include_str!("fixtures/openrouter-qwen3.8-27b.json")), 7).await;
    assert_eq!(counts.keys().collect::<Vec<_>>(), vec![&OUTCOME_PASSED], "{counts:?}");
}

#[tokio::test]
async fn recorded_qwen36_fails_every_draw() {
    let counts = outcomes(load("or-3.6", include_str!("fixtures/openrouter-qwen3.6-27b-alibaba.json")), 7).await;
    assert_eq!(counts.keys().collect::<Vec<_>>(), vec![&OUTCOME_FAILED], "{counts:?}");
}

#[tokio::test]
async fn recorded_qwen35_fails_every_draw() {
    let counts = outcomes(load("or-3.5", include_str!("fixtures/openrouter-qwen3.5-27b-alibaba.json")), 7).await;
    assert_eq!(counts.keys().collect::<Vec<_>>(), vec![&OUTCOME_FAILED], "{counts:?}");
}

#[tokio::test]
async fn recorded_qwen36_without_logprobs_fails_or_waits_for_review_never_passes() {
    let counts = outcomes(load("or-3.6-chutes", include_str!("fixtures/openrouter-qwen3.6-27b-chutes.json")), 7).await;
    assert!(!counts.contains_key(OUTCOME_PASSED), "{counts:?}");
    assert!(counts.keys().all(|o| *o == OUTCOME_FAILED || *o == OUTCOME_NEEDS_REVIEW), "{counts:?}");
}

#[tokio::test]
async fn the_fixed_set_alone_separates_qwen38_from_its_siblings() {
    let reference = spark();
    let config = VerificationConfig::default();
    let run = |candidate: Arc<dyn Provider>| {
        let reference = reference.clone();
        let config = config.clone();
        async move { run_probes(fixed_probes(), &candidate, "m", &reference, "m", &config).await }
    };
    let same = run(load("or-3.8", include_str!("fixtures/openrouter-qwen3.8-27b.json"))).await;
    let older = run(load("or-3.6", include_str!("fixtures/openrouter-qwen3.6-27b-alibaba.json"))).await;
    assert_eq!(same.scores.divergence_hard, 0, "{}", same.reason);
    assert!(same.scores.mean_divergence.unwrap() < 0.5);
    assert!(older.scores.divergence_hard >= 2, "{:?}", older.scores);
    let hard: Vec<&str> = older.results.iter().filter(|r| r.divergence.unwrap_or(0.0) > 4.0).map(|r| r.id.as_str()).collect();
    assert!(hard.contains(&"cutoff") && hard.contains(&"latest-rust"), "{hard:?}");
}
