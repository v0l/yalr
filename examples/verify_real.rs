use std::sync::Arc;

use yalr::db::ProviderType;
use yalr::providers::{create_provider, Provider};
use yalr::verify::{draw_probes, fixed_probes, pool_probes, run_probes, settings};

struct Side {
    provider: Arc<dyn Provider>,
    model: String,
}

fn side(prefix: &str) -> Side {
    let var = |name: &str| std::env::var(format!("{prefix}_{name}")).unwrap_or_else(|_| panic!("{prefix}_{name}"));
    let key = std::env::var(format!("{prefix}_KEY")).ok().filter(|k| !k.is_empty());
    let provider = create_provider(prefix, Some(prefix), &var("URL"), key.as_deref(), ProviderType::OpenAi).unwrap();
    Side { provider, model: var("MODEL") }
}

fn opt(value: Option<f32>) -> String {
    value.map_or("-".to_string(), |v| format!("{v:.3}"))
}

#[tokio::main]
async fn main() {
    let reference = side("REFERENCE");
    let candidate = side("CANDIDATE");
    let probes = if std::env::var("VERIFY_ALL").is_ok() {
        fixed_probes().into_iter().chain(pool_probes()).collect()
    } else {
        draw_probes(settings::get().random_probes)
    };
    let config = settings::get();
    let report = run_probes(probes, &candidate.provider, &candidate.model, &reference.provider, &reference.model, &config).await;
    println!("outcome={} reason={}", report.outcome, report.reason);
    println!("scores={}", serde_json::to_string(&report.scores).unwrap());
    println!("{:18} {:>5} {:>6} {:>7} {:>6} {:>7} {:>9} {:>5}  output", "probe", "fixed", "prefix", "gap", "top5", "diverge", "tokens", "task");
    for r in &report.results {
        println!(
            "{:18} {:>5} {:>6} {:>7} {:>6} {:>7} {:>4}/{:<4} {:>5}  {:?}",
            r.id,
            r.fixed,
            r.prefix_score,
            opt(r.logprob_gap),
            opt(r.top5_overlap),
            opt(r.divergence),
            r.prompt_tokens.map_or("-".into(), |t| t.to_string()),
            r.reference_prompt_tokens.map_or("-".into(), |t| t.to_string()),
            r.task_passed.map_or("-", |t| if t { "pass" } else { "fail" }),
            r.error.clone().unwrap_or_else(|| r.output.chars().take(40).collect()),
        );
    }
}
