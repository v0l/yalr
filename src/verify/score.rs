use serde::{Deserialize, Serialize};

use super::settings::VerificationConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenLogprob {
    pub token: String,
    pub logprob: f32,
    pub top: Vec<(String, f32)>,
}

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub text: String,
    pub tokens: Vec<TokenLogprob>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub id: String,
    pub fixed: bool,
    pub prompt: String,
    pub output: String,
    pub reference_output: String,
    pub agreed_tokens: u32,
    pub prefix_score: u32,
    pub identical: bool,
    pub logprob_gap: Option<f32>,
    pub top5_overlap: Option<f32>,
    #[serde(default)]
    pub divergence: Option<f32>,
    #[serde(default)]
    pub divergence_at: Option<u32>,
    #[serde(default)]
    pub candidate_logprobs: bool,
    pub prompt_tokens: Option<u32>,
    pub reference_prompt_tokens: Option<u32>,
    pub task_passed: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scores {
    pub probes: usize,
    pub median_prefix: f32,
    pub prompt_tokens_compared: usize,
    pub prompt_tokens_mismatched: usize,
    pub tasks_total: usize,
    pub tasks_failed: usize,
    pub mean_logprob_gap: Option<f32>,
    pub mean_top5_overlap: Option<f32>,
    #[serde(default)]
    pub min_prefix: Option<u32>,
    #[serde(default)]
    pub divergence_probes: usize,
    #[serde(default)]
    pub divergence_hard: usize,
    #[serde(default)]
    pub mean_divergence: Option<f32>,
    #[serde(default)]
    pub candidate_logprobs: bool,
    pub errors: usize,
}

#[derive(Debug, Clone)]
pub struct Judgement {
    pub passed: bool,
    pub inconclusive: bool,
    pub needs_review: bool,
    pub reason: String,
    pub scores: Scores,
}

fn approximate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

fn agreed_by_text(a: &str, b: &str) -> usize {
    let common = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    common.div_ceil(4)
}

pub fn compare(candidate: &Sample, reference: &Sample, max_tokens: u32) -> Comparison {
    let both_logprobs = !candidate.tokens.is_empty() && !reference.tokens.is_empty();
    let identical = candidate.text.trim() == reference.text.trim();
    if !both_logprobs {
        let agreed = if identical { approximate_tokens(&candidate.text) } else { agreed_by_text(&candidate.text, &reference.text) };
        return Comparison { agreed: agreed as u32, identical, logprob_gap: None, top5_overlap: None, max_tokens };
    }
    let agreed = candidate
        .tokens
        .iter()
        .zip(&reference.tokens)
        .take_while(|(a, b)| a.token == b.token)
        .count();
    let pairs: Vec<_> = candidate.tokens.iter().zip(&reference.tokens).take(agreed).collect();
    let gap = (!pairs.is_empty())
        .then(|| pairs.iter().map(|(a, b)| (a.logprob - b.logprob).abs()).sum::<f32>() / pairs.len() as f32);
    let overlaps: Vec<f32> = pairs
        .iter()
        .filter(|(a, b)| !a.top.is_empty() && !b.top.is_empty())
        .map(|(a, b)| {
            let shared = a.top.iter().filter(|(t, _)| b.top.iter().any(|(u, _)| u == t)).count();
            shared as f32 / a.top.len().max(b.top.len()) as f32
        })
        .collect();
    let top5_overlap = (!overlaps.is_empty()).then(|| overlaps.iter().sum::<f32>() / overlaps.len() as f32);
    Comparison { agreed: agreed as u32, identical, logprob_gap: gap, top5_overlap, max_tokens }
}

pub struct Comparison {
    pub agreed: u32,
    pub identical: bool,
    pub logprob_gap: Option<f32>,
    pub top5_overlap: Option<f32>,
    pub max_tokens: u32,
}

impl Comparison {
    pub fn prefix_score(&self) -> u32 {
        if self.identical {
            self.max_tokens
        } else {
            self.agreed.min(self.max_tokens)
        }
    }
}

fn median(mut values: Vec<f32>) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

fn mean(values: Vec<f32>) -> Option<f32> {
    (!values.is_empty()).then(|| values.iter().sum::<f32>() / values.len() as f32)
}

fn failures(results: &[&ProbeResult], scores: &Scores, config: &VerificationConfig) -> Vec<String> {
    let mut failures = Vec::new();
    if scores.prompt_tokens_mismatched > 0 {
        failures.push(format!(
            "prompt token counts differ from the reference on {} of {} prompts",
            scores.prompt_tokens_mismatched, scores.prompt_tokens_compared
        ));
    }
    if scores.tasks_failed > 0 {
        let names: Vec<&str> = results.iter().filter(|r| r.task_passed == Some(false)).map(|r| r.id.as_str()).collect();
        failures.push(format!("{} task probes failed ({})", scores.tasks_failed, names.join(", ")));
    }
    if scores.divergence_hard > config.max_hard_divergences {
        let names: Vec<&str> = results
            .iter()
            .filter(|r| r.divergence.is_some_and(|d| d > config.hard_divergence_nats))
            .map(|r| r.id.as_str())
            .collect();
        failures.push(format!(
            "{} probes left the reference's likely tokens by more than {} nats ({})",
            scores.divergence_hard, config.hard_divergence_nats, names.join(", ")
        ));
    }
    if let Some(mean) = scores.mean_divergence.filter(|m| *m > config.max_mean_divergence) {
        failures.push(format!("mean divergence {mean:.2} nats is over {}", config.max_mean_divergence));
    }
    if scores.candidate_logprobs {
        if let Some(gap) = scores.mean_logprob_gap.filter(|g| *g > config.max_logprob_gap) {
            failures.push(format!("logprob gap {gap:.2} is over {} nats", config.max_logprob_gap));
        }
        if let Some(min) = scores.min_prefix.filter(|m| *m < config.min_probe_prefix_tokens) {
            failures.push(format!("a probe agreed on only {min} tokens, under {}", config.min_probe_prefix_tokens));
        }
    }
    if scores.median_prefix < config.min_prefix_tokens as f32 {
        failures.push(format!(
            "median prefix agreement {:.1} is under {} tokens",
            scores.median_prefix, config.min_prefix_tokens
        ));
    }
    failures
}

pub fn judge(results: &[ProbeResult], config: &VerificationConfig) -> Judgement {
    let answered: Vec<&ProbeResult> = results.iter().filter(|r| r.error.is_none()).collect();
    let errors = results.len() - answered.len();
    let compared: Vec<&&ProbeResult> =
        answered.iter().filter(|r| r.fixed && r.prompt_tokens.is_some() && r.reference_prompt_tokens.is_some()).collect();
    let tasks: Vec<&&ProbeResult> = answered.iter().filter(|r| r.task_passed.is_some()).collect();
    let divergences: Vec<f32> = answered.iter().filter_map(|r| r.divergence).collect();
    let candidate_logprobs = !answered.is_empty() && answered.iter().all(|r| r.candidate_logprobs);
    let scores = Scores {
        probes: results.len(),
        median_prefix: median(answered.iter().map(|r| r.prefix_score as f32).collect()),
        prompt_tokens_compared: compared.len(),
        prompt_tokens_mismatched: compared.iter().filter(|r| r.prompt_tokens != r.reference_prompt_tokens).count(),
        tasks_total: tasks.len(),
        tasks_failed: tasks.iter().filter(|r| r.task_passed == Some(false)).count(),
        mean_logprob_gap: mean(answered.iter().filter_map(|r| r.logprob_gap).collect()),
        mean_top5_overlap: mean(answered.iter().filter_map(|r| r.top5_overlap).collect()),
        min_prefix: answered.iter().filter(|r| r.task_passed.is_some()).map(|r| r.prefix_score).min(),
        divergence_probes: divergences.len(),
        divergence_hard: divergences.iter().filter(|d| **d > config.hard_divergence_nats).count(),
        mean_divergence: mean(divergences),
        candidate_logprobs,
        errors,
    };

    if answered.len() * 2 < results.len() || answered.is_empty() {
        return Judgement {
            passed: false,
            inconclusive: true,
            needs_review: false,
            reason: format!("{errors} of {} probes errored", results.len()),
            scores,
        };
    }

    let failures = failures(&answered, &scores, config);
    if !failures.is_empty() {
        return Judgement { passed: false, inconclusive: false, needs_review: false, reason: failures.join("; "), scores };
    }
    let reference_logprobs = scores.divergence_probes * 2 >= answered.len();
    if reference_logprobs && !candidate_logprobs {
        return Judgement {
            passed: false,
            inconclusive: false,
            needs_review: true,
            reason: "the provider returns no logprobs, so a sibling model cannot be ruled out; an admin has to review it".to_string(),
            scores,
        };
    }
    Judgement { passed: true, inconclusive: false, needs_review: false, reason: String::new(), scores }
}
