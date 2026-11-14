use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::divergence::divergence;
use super::probes::{draw_probes, Probe};
use super::score::{compare, judge, ProbeResult, Sample, Scores, TokenLogprob};
use super::settings::VerificationConfig;
use super::{OUTCOME_ERROR, OUTCOME_FAILED, OUTCOME_NEEDS_REVIEW, OUTCOME_PASSED, OUTCOME_REFERENCE_ERROR};
use crate::providers::{ChatRequest, Provider};

#[derive(Debug, Clone)]
pub struct ModelReport {
    pub model: String,
    pub outcome: &'static str,
    pub passed: bool,
    pub reason: String,
    pub scores: Scores,
    pub results: Vec<ProbeResult>,
    pub reference_provider_id: Option<i64>,
}

impl ModelReport {
    pub fn without_result(model: &str, outcome: &'static str, reason: String) -> Self {
        Self {
            model: model.to_string(),
            outcome,
            passed: false,
            reason,
            scores: Scores::default(),
            results: vec![],
            reference_provider_id: None,
        }
    }

    pub fn is_conclusive(&self) -> bool {
        self.outcome == OUTCOME_PASSED || self.outcome == OUTCOME_FAILED || self.outcome == OUTCOME_NEEDS_REVIEW
    }
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct ProbeBody<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    temperature: f32,
    seed: i64,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    logprobs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_logprobs: Option<u8>,
    #[serde(flatten)]
    extras: &'a std::collections::BTreeMap<String, serde_json::Value>,
}

fn request(model: &str, probe: &Probe, config: &VerificationConfig, with_logprobs: bool) -> Result<ChatRequest, String> {
    let mut messages = Vec::new();
    if let Some(system) = probe.system.as_deref() {
        messages.push(Message { role: "system", content: system });
    }
    messages.push(Message { role: "user", content: &probe.prompt });
    let body = ProbeBody {
        model,
        messages,
        temperature: 0.0,
        seed: 1,
        max_tokens: config.probe_max_tokens,
        logprobs: with_logprobs.then_some(true),
        top_logprobs: with_logprobs.then_some(5),
        extras: &config.request_extras,
    };
    let value = serde_json::to_value(&body).map_err(|e| e.to_string())?;
    ChatRequest::from_value(value).map_err(|e| e.to_string())
}

fn sample_of(response: &crate::providers::ChatResponse) -> Sample {
    let inner = response.inner();
    let choice = inner.choices.first();
    let text = choice.and_then(|c| c.message.content.clone()).unwrap_or_default();
    let tokens = choice
        .and_then(|c| c.logprobs.as_ref())
        .and_then(|l| l.content.as_ref())
        .map(|content| {
            content
                .iter()
                .map(|t| TokenLogprob {
                    token: t.token.clone(),
                    logprob: t.logprob,
                    top: t.top_logprobs.iter().map(|top| (top.token.clone(), top.logprob)).collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    Sample {
        text,
        tokens,
        prompt_tokens: inner.usage.as_ref().map(|u| u.prompt_tokens),
        completion_tokens: inner.usage.as_ref().map(|u| u.completion_tokens),
        truncated: choice.and_then(|c| c.finish_reason).is_some_and(|r| r == async_openai::types::chat::FinishReason::Length),
    }
}

async fn sample(
    provider: &Arc<dyn Provider>,
    model: &str,
    probe: &Probe,
    config: &VerificationConfig,
) -> Result<Sample, String> {
    let timeout = Duration::from_secs(config.request_timeout_secs);
    let mut last = String::new();
    for with_logprobs in [true, false] {
        let req = request(model, probe, config, with_logprobs)?;
        match tokio::time::timeout(timeout, provider.chat_completions(&req)).await {
            Ok(Ok(response)) => return Ok(sample_of(&response)),
            Ok(Err(e)) => last = e.to_string(),
            Err(_) => last = "timed out".to_string(),
        }
    }
    Err(last)
}

struct Side {
    sample: Option<Sample>,
    error: Option<String>,
}

async fn side(provider: &Arc<dyn Provider>, model: &str, probe: &Probe, config: &VerificationConfig) -> Side {
    match sample(provider, model, probe, config).await {
        Ok(s) => Side { sample: Some(s), error: None },
        Err(e) => Side { sample: None, error: Some(e) },
    }
}

async fn run_probe(
    probe: Probe,
    candidate: &Arc<dyn Provider>,
    candidate_model: &str,
    reference: &Arc<dyn Provider>,
    reference_model: &str,
    config: &VerificationConfig,
) -> (ProbeResult, bool) {
    let (cand, refr) = futures::join!(
        side(candidate, candidate_model, &probe, config),
        side(reference, reference_model, &probe, config)
    );
    let reference_failed = refr.error.is_some();
    let mut result = ProbeResult {
        id: probe.id.clone(),
        fixed: probe.fixed,
        prompt: probe.prompt.clone(),
        output: String::new(),
        reference_output: String::new(),
        agreed_tokens: 0,
        prefix_score: 0,
        identical: false,
        logprob_gap: None,
        top5_overlap: None,
        divergence: None,
        divergence_at: None,
        candidate_logprobs: false,
        prompt_tokens: None,
        reference_prompt_tokens: None,
        task_passed: None,
        error: cand.error.clone().or_else(|| refr.error.clone().map(|e| format!("reference: {e}"))),
    };
    if let (Some(c), Some(r)) = (&cand.sample, &refr.sample) {
        let cmp = compare(c, r, config.probe_max_tokens);
        result.prefix_score = cmp.prefix_score();
        result.agreed_tokens = cmp.agreed;
        result.identical = cmp.identical;
        result.logprob_gap = cmp.logprob_gap;
        result.top5_overlap = cmp.top5_overlap;
        if let Some(d) = divergence(c, r) {
            result.divergence = Some(d.cost);
            result.divergence_at = Some(d.position);
        }
        result.candidate_logprobs = !c.tokens.is_empty();
        result.prompt_tokens = c.prompt_tokens;
        result.reference_prompt_tokens = r.prompt_tokens;
        result.output = c.text.clone();
        result.reference_output = r.text.clone();
        result.task_passed = probe.task.as_ref().map(|t| t.passes(&c.text));
    } else if let Some(c) = &cand.sample {
        result.output = c.text.clone();
    }
    (result, reference_failed)
}

pub async fn verify_model(
    candidate: &Arc<dyn Provider>,
    candidate_model: &str,
    reference: &Arc<dyn Provider>,
    reference_model: &str,
    config: &VerificationConfig,
) -> ModelReport {
    run_probes(draw_probes(config.random_probes), candidate, candidate_model, reference, reference_model, config).await
}

pub async fn run_probes(
    probes: Vec<Probe>,
    candidate: &Arc<dyn Provider>,
    candidate_model: &str,
    reference: &Arc<dyn Provider>,
    reference_model: &str,
    config: &VerificationConfig,
) -> ModelReport {
    let outcomes = futures::future::join_all(probes.into_iter().map(|probe| {
        run_probe(probe, candidate, candidate_model, reference, reference_model, config)
    }))
    .await;
    report_from(candidate_model, outcomes, config)
}

fn report_from(candidate_model: &str, outcomes: Vec<(ProbeResult, bool)>, config: &VerificationConfig) -> ModelReport {
    let reference_failures = outcomes.iter().filter(|(_, failed)| *failed).count();
    let results: Vec<ProbeResult> = outcomes.into_iter().map(|(r, _)| r).collect();
    if reference_failures * 2 > results.len() {
        let mut report = ModelReport::without_result(
            candidate_model,
            OUTCOME_REFERENCE_ERROR,
            format!("the reference provider failed {reference_failures} of {} probes", results.len()),
        );
        report.results = results;
        return report;
    }
    let judgement = judge(&results, config);
    let outcome = if judgement.inconclusive {
        OUTCOME_ERROR
    } else if judgement.needs_review {
        OUTCOME_NEEDS_REVIEW
    } else if judgement.passed {
        OUTCOME_PASSED
    } else {
        OUTCOME_FAILED
    };
    ModelReport {
        model: candidate_model.to_string(),
        outcome,
        passed: judgement.passed,
        reason: judgement.reason,
        scores: judgement.scores,
        results,
        reference_provider_id: None,
    }
}
