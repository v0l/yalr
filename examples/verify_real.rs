use std::sync::Arc;

use yalr::db::ProviderType;
use yalr::providers::{create_provider, Provider};
use yalr::verify::{settings, verify_model};

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

#[tokio::main]
async fn main() {
    let reference = side("REFERENCE");
    let candidate = side("CANDIDATE");
    let report = verify_model(&candidate.provider, &candidate.model, &reference.provider, &reference.model, &settings::get()).await;
    println!("outcome={} passed={} reason={}", report.outcome, report.passed, report.reason);
    println!("scores={}", serde_json::to_string(&report.scores).unwrap());
    for r in &report.results {
        println!(
            "{:16} fixed={:5} prefix={:2} tokens={:?}/{:?} task={:?} gap={:?} out={:?} ref={:?} err={:?}",
            r.id, r.fixed, r.prefix_score, r.prompt_tokens, r.reference_prompt_tokens, r.task_passed,
            r.logprob_gap, r.output.chars().take(40).collect::<String>(), r.reference_output.chars().take(40).collect::<String>(), r.error
        );
    }
}
