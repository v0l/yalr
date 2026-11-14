use serde::{Deserialize, Serialize};

use super::settings::VerificationConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenLogprob {
    pub token: String,
    pub logprob: f32,
    pub top: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub text: String,
    pub tokens: Vec<TokenLogprob>,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
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
    pub errors: usize,
}

#[derive(Debug, Clone)]
pub struct Judgement {
    pub passed: bool,
    pub inconclusive: bool,
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
            let shared = a.top.iter().filter(|t| b.top.contains(t)).count();
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

pub fn judge(results: &[ProbeResult], config: &VerificationConfig) -> Judgement {
    let answered: Vec<&ProbeResult> = results.iter().filter(|r| r.error.is_none()).collect();
    let errors = results.len() - answered.len();
    let compared: Vec<&&ProbeResult> =
        answered.iter().filter(|r| r.fixed && r.prompt_tokens.is_some() && r.reference_prompt_tokens.is_some()).collect();
    let mismatched = compared.iter().filter(|r| r.prompt_tokens != r.reference_prompt_tokens).count();
    let tasks: Vec<&&ProbeResult> = answered.iter().filter(|r| r.task_passed.is_some()).collect();
    let tasks_failed = tasks.iter().filter(|r| r.task_passed == Some(false)).count();
    let scores = Scores {
        probes: results.len(),
        median_prefix: median(answered.iter().map(|r| r.prefix_score as f32).collect()),
        prompt_tokens_compared: compared.len(),
        prompt_tokens_mismatched: mismatched,
        tasks_total: tasks.len(),
        tasks_failed,
        mean_logprob_gap: mean(answered.iter().filter_map(|r| r.logprob_gap).collect()),
        mean_top5_overlap: mean(answered.iter().filter_map(|r| r.top5_overlap).collect()),
        errors,
    };

    if answered.len() * 2 < results.len() || answered.is_empty() {
        return Judgement {
            passed: false,
            inconclusive: true,
            reason: format!("{errors} of {} probes errored", results.len()),
            scores,
        };
    }

    let mut failures = Vec::new();
    if mismatched > 0 {
        failures.push(format!("prompt token counts differ from the reference on {mismatched} of {} prompts", compared.len()));
    }
    if tasks_failed > 0 {
        let names: Vec<&str> = tasks.iter().filter(|r| r.task_passed == Some(false)).map(|r| r.id.as_str()).collect();
        failures.push(format!("{tasks_failed} task probes failed ({})", names.join(", ")));
    }
    if scores.median_prefix < config.min_prefix_tokens as f32 {
        failures.push(format!(
            "median prefix agreement {:.1} is under {} tokens",
            scores.median_prefix, config.min_prefix_tokens
        ));
    }
    if let Some(gap) = scores.mean_logprob_gap.filter(|g| *g >= config.max_logprob_gap) {
        failures.push(format!("logprob gap {gap:.2} is not under {} nats", config.max_logprob_gap));
    }
    Judgement { passed: failures.is_empty(), inconclusive: false, reason: failures.join("; "), scores }
}
