use super::score::{Sample, TokenLogprob};

pub const NOT_IN_TOP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Divergence {
    pub position: u32,
    pub cost: f32,
}

fn is_end(token: &str) -> bool {
    matches!(token, "</s>" | "<eos>" | "<end_of_turn>") || (token.starts_with("<|") && token.ends_with("|>"))
}

fn best_alternative<'a>(rest: &str, step: &'a TokenLogprob) -> Option<&'a (String, f32)> {
    step.top
        .iter()
        .filter(|(token, _)| !token.is_empty() && !is_end(token) && rest.starts_with(token.as_str()))
        .max_by_key(|(token, _)| token.len())
}

fn end_alternative(step: &TokenLogprob) -> Option<f32> {
    step.top.iter().filter(|(token, _)| is_end(token)).map(|(_, lp)| *lp).reduce(f32::max)
}

pub fn divergence(candidate: &Sample, reference: &Sample) -> Option<Divergence> {
    if reference.tokens.is_empty() {
        return None;
    }
    let mut rest = candidate.text.as_str();
    for (i, step) in reference.tokens.iter().enumerate() {
        let position = i as u32;
        if is_end(&step.token) {
            let cost = if rest.trim().is_empty() {
                0.0
            } else {
                best_alternative(rest, step).map_or(NOT_IN_TOP, |(_, lp)| step.logprob - lp)
            };
            return Some(Divergence { position, cost });
        }
        if let Some(after) = rest.strip_prefix(step.token.as_str()) {
            rest = after;
            continue;
        }
        if rest.is_empty() {
            let cost = if candidate.truncated {
                0.0
            } else {
                end_alternative(step).map_or(NOT_IN_TOP, |lp| (step.logprob - lp).max(0.0))
            };
            return Some(Divergence { position, cost });
        }
        let cost = best_alternative(rest, step).map_or(NOT_IN_TOP, |(_, lp)| (step.logprob - lp).max(0.0));
        return Some(Divergence { position, cost });
    }
    Some(Divergence { position: reference.tokens.len() as u32, cost: 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(token: &str, logprob: f32, top: &[(&str, f32)]) -> TokenLogprob {
        TokenLogprob {
            token: token.to_string(),
            logprob,
            top: top.iter().map(|(t, l)| (t.to_string(), *l)).collect(),
        }
    }

    fn reference() -> Sample {
        Sample {
            text: "I am Qwen.".into(),
            tokens: vec![
                step("I", -0.01, &[("I", -0.01), ("Hello", -3.0)]),
                step(" am", -0.1, &[(" am", -0.1), ("'m", -2.5)]),
                step(" Qwen", -0.2, &[(" Qwen", -0.2), (" a", -1.0)]),
                step(".", -0.3, &[(".", -0.3), (",", -1.2)]),
                step("<|im_end|>", -0.01, &[("<|im_end|>", -0.01), ("\n", -6.0)]),
            ],
            ..Default::default()
        }
    }

    fn candidate(text: &str) -> Sample {
        Sample { text: text.into(), ..Default::default() }
    }

    #[test]
    fn identical_text_has_no_cost() {
        assert_eq!(divergence(&candidate("I am Qwen."), &reference()), Some(Divergence { position: 4, cost: 0.0 }));
    }

    #[test]
    fn a_plausible_alternative_costs_its_logprob_deficit() {
        let d = divergence(&candidate("I am a model"), &reference()).unwrap();
        assert_eq!(d.position, 2);
        assert!((d.cost - 0.8).abs() < 1e-5);
    }

    #[test]
    fn a_token_outside_the_top_five_costs_the_ceiling() {
        let d = divergence(&candidate("I am Llama."), &reference()).unwrap();
        assert_eq!(d, Divergence { position: 2, cost: NOT_IN_TOP });
    }

    #[test]
    fn stopping_early_without_end_in_the_top_five_costs_the_ceiling() {
        assert_eq!(divergence(&candidate("I am"), &reference()).unwrap().cost, NOT_IN_TOP);
        assert_eq!(divergence(&candidate(""), &reference()).unwrap().cost, NOT_IN_TOP);
    }

    #[test]
    fn a_length_cut_is_not_held_against_the_candidate() {
        let cut = Sample { text: "I am".into(), truncated: true, ..Default::default() };
        assert_eq!(divergence(&cut, &reference()).unwrap().cost, 0.0);
    }

    #[test]
    fn running_on_past_the_reference_end_is_scored_at_the_end_token() {
        let d = divergence(&candidate("I am Qwen.\nMore"), &reference()).unwrap();
        assert_eq!(d.position, 4);
        assert!((d.cost - 5.99).abs() < 1e-4);
    }
}
